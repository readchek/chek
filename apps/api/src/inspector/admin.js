import {
  action,
  capabilities,
  disclosure,
  heading,
  json,
  link,
  node,
  notice,
  post,
  raw,
  route,
  showDetail,
  state,
  table,
} from "./ui.js"

function field(form, labelText, id, type = "text") {
  const label = node("label", labelText)
  label.htmlFor = id
  const input = node(type === "textarea" ? "textarea" : "input")
  input.id = id
  if (type !== "textarea") input.type = type
  form.append(label, input)
  return input
}

export function sources(container, _params, signal, refresh) {
  container.append(
    heading(
      "Sources",
      "Connect a library to browse it. Import originals when you want a local copy in Chek.",
    ),
  )
  const { wrap, body } = table(["Source", "Extension", "Actions"])
  for (const source of state.sources) {
    const row = node("tr")
    row.append(node("td", source.name), node("td", source.extensionId))
    const actions = node("td")
    const bar = node("div", undefined, "hstack")
    const caps = capabilities(source)
    if (caps.includes("browse") || caps.includes("search"))
      bar.append(link("Browse", route("browse", { source: source.id }), "button small outline"))
    if (caps.includes("discover"))
      bar.append(
        action(
          "Import originals",
          async () => {
            await post(`/api/v1/sources/${source.id}/scan`, undefined, signal)
            location.hash = "jobs"
          },
          "small outline",
        ),
      )
    actions.append(bar)
    row.append(actions)
    body.append(row)
  }
  container.append(state.sources.length ? wrap : node("p", "No sources connected yet."))
  const section = node("section", undefined, "card mt-6")
  section.append(node("h2", "Add a source"))
  const form = node("form")
  const name = field(form, "Name", "source-name")
  name.required = true
  name.maxLength = 256
  name.placeholder = "My library"
  const label = node("label", "Extension")
  label.htmlFor = "extension-select"
  const select = node("select")
  select.id = label.htmlFor
  for (const extension of state.extensions) {
    const option = node("option", extension.name)
    option.value = extension.id
    select.append(option)
  }
  const config = node("div")
  const help = node("p", undefined, "text-light")
  const grants = node("div")
  grants.append(
    node(
      "p",
      "Folders and origins from the fields above are granted automatically. Add only extra locations the extension needs.",
    ),
  )
  const roots = field(
    grants,
    "Extra folders (one absolute path per line)",
    "read-roots",
    "textarea",
  )
  const origins = field(grants, "Extra HTTP origins (one per line)", "http-origins", "textarea")
  const submit = node("button", "Connect source")
  form.append(label, select, help, config, disclosure("Advanced permissions", grants), submit)
  const renderConfig = () => {
    config.replaceChildren()
    const extension = state.extensions.find((entry) => entry.id === select.value)
    help.textContent = extension?.description || "No extensions installed."
    submit.disabled = !extension
    for (const entry of extension?.configFields ?? []) {
      const input = field(
        config,
        entry.label,
        `config-${entry.key}`,
        entry.secret ? "password" : "text",
      )
      input.name = entry.key
      input.required = entry.required
      input.autocomplete = "off"
    }
  }
  select.addEventListener("change", renderConfig)
  renderConfig()
  form.addEventListener("submit", async (event) => {
    event.preventDefault()
    submit.disabled = true
    submit.textContent = "Connecting…"
    try {
      const extension = state.extensions.find((entry) => entry.id === select.value)
      const values = Object.fromEntries(
        [...config.querySelectorAll("input")].map((input) => [input.name, input.value]),
      )
      const lines = (input) =>
        input.value
          .split("\n")
          .map((line) => line.trim())
          .filter(Boolean)
      const source = await post(
        "/api/v1/sources",
        {
          name: name.value,
          extensionId: select.value,
          config: values,
          readRoots: [
            ...new Set([
              ...lines(roots),
              ...extension.configFields
                .filter((entry) => entry.grant === "read-directory")
                .map((entry) => values[entry.key])
                .filter(Boolean),
            ]),
          ],
          httpOrigins: [
            ...new Set([
              ...lines(origins),
              ...extension.configFields
                .filter((entry) => entry.grant === "http-origin" && values[entry.key])
                .map((entry) => new URL(values[entry.key]).origin),
            ]),
          ],
        },
        signal,
      )
      await refresh(signal)
      notice("Source connected. Imports can be started from Manage sources.")
      if (extension.capabilities.includes("browse") || extension.capabilities.includes("search"))
        location.hash = route("browse", { source: source.id })
      else {
        container.replaceChildren()
        sources(container, new URLSearchParams(), signal, refresh)
      }
    } catch (error) {
      if (error.name !== "AbortError") notice(error.message, true)
    } finally {
      submit.disabled = false
      submit.textContent = "Connect source"
    }
  })
  section.append(form)
  container.append(section)
}

export async function jobs(container, _params, signal) {
  container.append(
    heading(
      "Import jobs",
      "Updates every three seconds while this view is open. Open a job to inspect its events.",
    ),
  )
  const { wrap, body } = table(["Source", "State", "Imported", "Unchanged", "Warnings", "Actions"])
  const empty = node("p", "No import jobs yet.")
  container.append(wrap, empty)
  let snapshot = ""
  let timer
  const load = async () => {
    const data = await json("/api/v1/jobs", { signal })
    signal.throwIfAborted()
    const next = JSON.stringify(data)
    if (next === snapshot) return
    snapshot = next
    body.replaceChildren()
    empty.hidden = !!data.length
    wrap.hidden = !data.length
    for (const job of data) {
      const row = node("tr")
      row.append(
        node(
          "td",
          state.sources.find((source) => source.id === job.sourceId)?.name || job.sourceId,
        ),
      )
      const status = node("td")
      const badge = node("span", job.state, "badge outline")
      badge.dataset.variant =
        job.state === "failed" ? "danger" : job.state === "completed" ? "success" : "secondary"
      status.append(badge)
      row.append(status)
      for (const value of [job.imported, job.skipped, job.warnings])
        row.append(node("td", String(value)))
      const cell = node("td")
      const actions = node("div", undefined, "hstack")
      for (const [label, states, endpoint] of [
        ["Cancel", ["queued", "running"], "cancel"],
        ["Retry", ["failed", "cancelled"], "retry"],
      ]) {
        if (states.includes(job.state))
          actions.append(
            action(
              label,
              async () => {
                await post(`/api/v1/jobs/${job.id}/${endpoint}`, undefined, signal)
                await load()
              },
              "small outline",
            ),
          )
      }
      actions.append(
        action(
          "Events",
          async () => {
            const events = await json(`/api/v1/jobs/${job.id}/events`, { signal })
            signal.throwIfAborted()
            const details = node("div")
            details.append(
              node(
                "p",
                `${job.state} · attempt ${job.attempts} · ${job.imported} imported · ${job.skipped} unchanged`,
              ),
            )
            if (job.errorMessage) details.append(node("p", `${job.errorCode}: ${job.errorMessage}`))
            const eventList = node("ul")
            for (const event of events) {
              const item = node("li")
              item.append(
                node("strong", event.code),
                node("p", event.message),
                node("small", new Date(event.createdAt).toLocaleString(), "text-light"),
              )
              eventList.append(item)
            }
            details.append(events.length ? eventList : node("p", "No warnings or errors recorded."))
            details.append(disclosure("Job data & trace", raw({ ...job, events })))
            showDetail("Import job", details)
          },
          "small outline",
        ),
      )
      cell.append(actions)
      row.append(cell)
      body.append(row)
    }
  }
  const poll = async () => {
    try {
      if (!document.hidden) await load()
    } catch (error) {
      if (error.name !== "AbortError") notice(error.message, true)
    } finally {
      if (!signal.aborted) timer = setTimeout(poll, 3000)
    }
  }
  signal.addEventListener("abort", () => clearTimeout(timer), { once: true })
  await load()
  timer = setTimeout(poll, 3000)
}

export async function extensions(container, _params, signal) {
  container.append(
    heading(
      "Extensions",
      "Installed extensions run as trusted code. Sources share a common interface regardless of where their books live.",
    ),
  )
  const list = node("div", undefined, "vstack")
  for (const extension of state.extensions) {
    const article = node("article", undefined, "card")
    article.append(
      node("h2", extension.name),
      node("p", extension.description),
      node(
        "small",
        `${extension.id} · ${extension.version} · protocol ${extension.protocolVersion}`,
        "text-light",
      ),
    )
    const badges = node("div", undefined, "hstack mt-4")
    for (const capability of extension.capabilities)
      badges.append(node("span", capability, "badge outline"))
    article.append(badges)
    list.append(article)
  }
  container.append(list)
  const collections = await json("/api/v1/extension-collections", { signal })
  signal.throwIfAborted()
  if (collections.length) container.append(node("h2", "Collections", "mt-6"))
  for (const collection of collections)
    container.append(disclosure(`${collection.name} · ${collection.author.name}`, raw(collection)))
}

export function access(container, _params, _signal, refresh) {
  container.append(
    heading(
      "Server access",
      "Set an API token if this server requires one. The token stays in this browser tab’s session.",
    ),
  )
  const form = node("form")
  const token = field(form, "API token", "token", "password")
  token.autocomplete = "off"
  token.value = state.token
  const button = node("button", "Connect")
  form.append(button)
  form.addEventListener("submit", async (event) => {
    event.preventDefault()
    button.disabled = true
    state.token = token.value.trim()
    sessionStorage.setItem("chek-token", state.token)
    try {
      await refresh()
      notice("Connected")
      location.hash = "library"
    } catch (error) {
      notice(error.message, true)
    } finally {
      button.disabled = false
    }
  })
  container.append(form)
}
