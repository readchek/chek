const element = (id) => document.getElementById(id)
let token = sessionStorage.getItem("chek-token") || ""
let extensions = []
let sources = []
let nextCursor = null
let loading = false
let jobSnapshot = ""

function node(tag, text) {
  const result = document.createElement(tag)
  if (text !== undefined) result.textContent = text
  return result
}

function action(label, run) {
  const button = node("button", label)
  button.type = "button"
  button.addEventListener("click", async () => {
    button.disabled = true
    try {
      await run()
    } catch (error) {
      notice(error.message)
    } finally {
      button.disabled = false
    }
  })
  return button
}

function notice(message) {
  element("notice").textContent = message
}

async function api(path, options = {}) {
  const response = await fetch(path, {
    ...options,
    headers: {
      ...(options.body ? { "Content-Type": "application/json" } : {}),
      ...(token ? { Authorization: `Bearer ${token}` } : {}),
      ...options.headers,
    },
  })
  if (!response.ok) {
    const error = await response.json().catch(() => ({}))
    throw new Error(
      `${error.message || error.code || `HTTP ${response.status}`}${response.headers.get("x-trace-id") ? ` · trace ${response.headers.get("x-trace-id")}` : ""}`,
    )
  }
  return response
}

async function json(path, options) {
  return (await api(path, options)).json()
}
const post = (path, payload) =>
  json(path, { method: "POST", ...(payload ? { body: JSON.stringify(payload) } : {}) })

function showDetail(title, content) {
  releasePageObjects()
  element("detail-title").textContent = title
  element("detail-content").replaceChildren(content)
  if (!element("detail").open) element("detail").showModal()
}

function raw(value) {
  const pre = node("pre")
  pre.append(node("code", JSON.stringify(value, null, 2)))
  return pre
}

function renderConfig() {
  const extension = extensions.find((entry) => entry.id === element("extension-select").value)
  const container = element("config-fields")
  container.replaceChildren()
  if (!extension) return
  for (const field of extension.configFields) {
    const label = node("label", field.label)
    const input = node("input")
    input.id = `config-${field.key}`
    label.htmlFor = input.id
    input.name = field.key
    input.required = field.required
    input.type = field.secret ? "password" : "text"
    input.autocomplete = "off"
    container.append(label, input)
  }
  element("grants").hidden = false
}

async function loadExtensions() {
  extensions = await json("/api/v1/extensions")
  element("extension-select").replaceChildren(
    ...extensions.map((extension) => {
      const option = node("option", extension.name)
      option.value = extension.id
      return option
    }),
  )
  element("extension-list").replaceChildren(
    ...extensions.map((extension) => {
      const article = node("article")
      article.append(
        node("h3", `${extension.name} · ${extension.version}`),
        node("p", extension.description),
        node("p", `Protocol ${extension.protocolVersion} · ${extension.capabilities.join(", ")}`),
      )
      return article
    }),
  )
  const collections = await json("/api/v1/extension-collections")
  for (const collection of collections) {
    const article = node("article")
    article.append(node("h3", collection.name), node("p", `By ${collection.author.name}`))
    const releases = node("ul")
    for (const release of collection.extensions) {
      releases.append(node("li", `${release.package}@${release.version} · ${release.id}`))
    }
    article.append(
      releases,
      node("p", "Collection listing; packages must be installed explicitly."),
    )
    element("extension-list").append(article)
  }
  renderConfig()
}

async function loadSources() {
  sources = await json("/api/v1/sources")
  element("source-list").replaceChildren(
    ...(sources.length
      ? sources.map((source) => {
          const line = node("p")
          line.append(
            node("strong", source.name),
            document.createTextNode(` · ${source.extensionId} `),
          )
          const capabilities =
            extensions.find((extension) => extension.id === source.extensionId)?.capabilities ?? []
          if (capabilities.includes("discover"))
            line.append(
              action("Scan", async () => {
                await post(`/api/v1/sources/${source.id}/scan`)
                notice("Import queued")
                await loadJobs()
              }),
            )
          if (capabilities.includes("browse") || capabilities.includes("search"))
            line.append(action("Browse", () => browseSource(source)))
          return line
        })
      : [node("p", "No sources yet. Add a folder to begin.")]),
  )
}

const querySource = (source, query) => post(`/api/v1/sources/${source.id}/query`, query)

async function browseSource(source) {
  const container = node("div")
  const form = node("form")
  const input = node("input")
  input.type = "search"
  input.placeholder = "Search this source"
  input.setAttribute("aria-label", "Search this source")
  const results = node("div")
  const capabilities = extensions.find(
    (extension) => extension.id === source.extensionId,
  ).capabilities
  const load = async (cursor) => {
    const text = input.value.trim()
    const operation = text ? "search" : "browse"
    if (!capabilities.includes(operation))
      throw new Error(`This source does not support ${operation}`)
    const result = await querySource(source, {
      operation,
      limit: 20,
      filters: {},
      ...(text ? { text } : {}),
      ...(cursor ? { cursor } : {}),
    })
    if (!cursor) results.replaceChildren()
    for (const series of result.data.items) {
      const article = node("article")
      article.append(
        node("h3", series.title),
        node("p", `${series.status} · ${series.languages.join(", ")}`),
      )
      if (capabilities.includes("chapters"))
        article.append(action("Chapters", () => showChapters(source, series)))
      if (capabilities.includes("series"))
        article.append(
          action("Metadata", async () =>
            showDetail(
              series.title,
              raw(await querySource(source, { operation: "series", seriesKey: series.key })),
            ),
          ),
        )
      results.append(article)
    }
    more.hidden = !result.data.nextCursor
    next = result.data.nextCursor
  }
  let next = null
  const more = action("More series", () => load(next))
  more.hidden = true
  form.append(input, node("button", "Search"))
  form.addEventListener("submit", (event) => {
    event.preventDefault()
    load().catch((error) => notice(error.message))
  })
  container.append(form, results, more)
  showDetail(source.name, container)
  if (capabilities.includes("browse")) await load()
}

async function showChapters(source, series) {
  const container = node("div")
  container.append(action("Back to source", () => browseSource(source)))
  const list = node("ol")
  let next = null
  const more = action("More chapters", () => load(next))
  const load = async (cursor) => {
    const result = await querySource(source, {
      operation: "chapters",
      seriesKey: series.key,
      limit: 30,
      ...(cursor ? { cursor } : {}),
    })
    for (const chapter of result.data.items) {
      const item = node("li")
      item.append(
        action(chapter.title, () => showPages(source, series, chapter)),
        node("small", ` ${chapter.language ?? ""} ${chapter.groups.join(", ")}`),
      )
      list.append(item)
    }
    next = result.data.nextCursor
    more.hidden = !next
  }
  container.append(list, more)
  showDetail(series.title, container)
  await load()
}

const pageObjects = new Set()
function releasePageObjects() {
  for (const url of pageObjects) URL.revokeObjectURL(url)
  pageObjects.clear()
}
element("detail").addEventListener("close", releasePageObjects)

async function showPages(source, series, chapter) {
  releasePageObjects()
  const container = node("div")
  container.append(action("Back to chapters", () => showChapters(source, series)))
  const pages = node("div")
  let next = null
  const more = action("More pages", () => load(next))
  const load = async (cursor) => {
    const result = await querySource(source, {
      operation: "pages",
      chapterKey: chapter.key,
      limit: 10,
      ...(cursor ? { cursor } : {}),
    })
    for (const page of result.data.items) {
      const figure = node("figure")
      const image = node("img")
      image.alt = `${chapter.title}, page ${page.index + 1}`
      image.loading = "lazy"
      image.className = "source-page"
      if (token) {
        const object = URL.createObjectURL(await (await api(page.asset)).blob())
        pageObjects.add(object)
        image.src = object
      } else image.src = page.asset
      image.addEventListener(
        "error",
        () => {
          figure.append(node("p", "Image could not load. Reopen the chapter to refresh its URLs."))
        },
        { once: true },
      )
      figure.append(image, node("figcaption", `Page ${page.index + 1}`))
      pages.append(figure)
    }
    next = result.data.nextCursor
    more.hidden = !next
  }
  container.append(pages, more)
  showDetail(chapter.title, container)
  await load()
}

async function publicationDetail(publication) {
  const detail = await json(`/api/v1/publications/${publication.id}`)
  const container = node("div")
  const link = node("a", "Download original")
  link.href = `/api/v1/assets/${detail.assetHash}`
  link.download = `${detail.title}.${detail.format}`
  if (!token) container.append(link)
  else
    container.append(
      node("p", "Original URL (requires your Authorization header):"),
      node("code", `${location.origin}/api/v1/assets/${detail.assetHash}`),
    )
  container.append(
    action("Check byte-range delivery", async () => {
      const response = await api(`/api/v1/assets/${detail.assetHash}`, {
        headers: { Range: "bytes=0-63" },
      })
      const bytes = new Uint8Array(await response.arrayBuffer())
      container.append(
        node(
          "p",
          `HTTP ${response.status} · ${response.headers.get("Content-Range")} · ${bytes.length} bytes`,
        ),
      )
    }),
    raw(detail),
  )
  showDetail(detail.title, container)
}

async function loadLibrary(append = false) {
  const query = new URLSearchParams({ search: element("search").value, limit: "30" })
  if (append && nextCursor) query.set("after", nextCursor)
  const data = await json(`/api/v1/publications?${query}`)
  if (!append) element("publications").replaceChildren()
  for (const publication of data.items) {
    const row = node("tr")
    for (const text of [
      publication.title,
      publication.format.toUpperCase(),
      `${(publication.byteLength / 1048576).toFixed(1)} MiB`,
      publication.status,
    ])
      row.append(node("td", text))
    const cell = node("td")
    cell.append(action("Inspect", () => publicationDetail(publication)))
    row.append(cell)
    element("publications").append(row)
  }
  nextCursor = data.nextCursor
  element("more").hidden = !nextCursor
  element("library-caption").textContent = element("publications").children.length
    ? "Imported publications"
    : "No matching publications"
}

async function loadJobs() {
  const jobs = await json("/api/v1/jobs")
  const snapshot = JSON.stringify(jobs)
  const changed = jobSnapshot !== snapshot
  jobSnapshot = snapshot
  if (!changed) return
  element("job-list").replaceChildren(
    ...jobs.map((job) => {
      const row = node("tr")
      for (const text of [
        sources.find((source) => source.id === job.sourceId)?.name ?? job.sourceId,
        job.state,
        job.imported,
        job.skipped,
        job.warnings,
      ])
        row.append(node("td", String(text)))
      const cell = node("td")
      if (["queued", "running"].includes(job.state))
        cell.append(
          action("Cancel", async () => {
            await post(`/api/v1/jobs/${job.id}/cancel`)
            await loadJobs()
          }),
        )
      if (["failed", "cancelled"].includes(job.state))
        cell.append(
          action("Retry", async () => {
            await post(`/api/v1/jobs/${job.id}/retry`)
            await loadJobs()
          }),
        )
      cell.append(
        action("Details", async () => {
          const events = await json(`/api/v1/jobs/${job.id}/events`)
          showDetail("Import job", raw({ ...job, events }))
        }),
      )
      row.append(cell)
      return row
    }),
  )
  await loadLibrary()
}

async function connect() {
  try {
    await loadExtensions()
    await loadSources()
    await loadJobs()
    await loadLibrary()
    notice("Connected")
  } catch (error) {
    notice(error.message)
  }
}

element("access-form").addEventListener("submit", async (event) => {
  event.preventDefault()
  token = element("token").value.trim()
  sessionStorage.setItem("chek-token", token)
  await connect()
})
element("source-form").addEventListener("submit", async (event) => {
  event.preventDefault()
  const button = event.submitter
  button.disabled = true
  try {
    const config = Object.fromEntries(
      [...element("config-fields").querySelectorAll("input")].map((input) => [
        input.name,
        input.value,
      ]),
    )
    const extensionId = element("extension-select").value
    const extension = extensions.find((entry) => entry.id === extensionId)
    const lines = (id) =>
      element(id)
        .value.split("\n")
        .map((line) => line.trim())
        .filter(Boolean)
    const source = await post("/api/v1/sources", {
      name: element("source-name").value,
      extensionId,
      config,
      readRoots: [
        ...new Set([
          ...lines("read-roots"),
          ...extension.configFields
            .filter((field) => field.grant === "read-directory")
            .map((field) => config[field.key])
            .filter(Boolean),
        ]),
      ],
      httpOrigins: [
        ...new Set([
          ...lines("http-origins"),
          ...extension.configFields
            .filter((field) => field.grant === "http-origin")
            .map((field) => new URL(config[field.key]).origin),
        ]),
      ],
    })
    if (extension.capabilities.includes("discover")) {
      await post(`/api/v1/sources/${source.id}/scan`)
      notice("Source added; import queued")
    } else notice("Source added; ready to browse")
    await loadSources()
    await loadJobs()
    event.target.reset()
    renderConfig()
  } catch (error) {
    notice(error.message)
  } finally {
    button.disabled = false
  }
})
element("extension-select").addEventListener("change", renderConfig)
element("search-form").addEventListener("submit", async (event) => {
  event.preventDefault()
  try {
    await loadLibrary()
  } catch (error) {
    notice(error.message)
  }
})
element("more").addEventListener("click", async () => {
  try {
    await loadLibrary(true)
  } catch (error) {
    notice(error.message)
  }
})
setInterval(async () => {
  if (document.hidden || loading) return
  loading = true
  try {
    await loadJobs()
  } catch (error) {
    notice(error.message)
  } finally {
    loading = false
  }
}, 3000)
connect()
