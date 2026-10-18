export const element = (id) => document.getElementById(id)
export const state = {
  extensions: [],
  sources: [],
  token: sessionStorage.getItem("chek-token") || "",
}

export function node(tag, text, className) {
  const result = document.createElement(tag)
  if (text !== undefined) result.textContent = text
  if (className) result.className = className
  return result
}

export function notice(message, error = false) {
  const target = element("notice")
  target.textContent = message
  target.hidden = !message
  target.setAttribute("role", error ? "alert" : "status")
}

export function action(label, run, className = "outline") {
  const button = node("button", label, className)
  button.type = "button"
  button.addEventListener("click", async () => {
    button.disabled = true
    try {
      await run()
    } catch (error) {
      if (error.name !== "AbortError") notice(error.message, true)
    } finally {
      button.disabled = false
    }
  })
  return button
}

export async function api(path, options = {}) {
  const response = await fetch(path, {
    ...options,
    headers: {
      ...(options.body ? { "Content-Type": "application/json" } : {}),
      ...(state.token ? { Authorization: `Bearer ${state.token}` } : {}),
      ...options.headers,
    },
  })
  if (!response.ok) {
    const error = await response.json().catch(() => ({}))
    const trace = response.headers.get("x-trace-id")
    throw new Error(
      `${[error.code, error.message].filter(Boolean).join(": ") || `HTTP ${response.status}`}${trace ? ` · trace ${trace}` : ""}`,
    )
  }
  return response
}

export async function json(path, options) {
  return (await api(path, options)).json()
}
export const post = (path, payload, signal) =>
  json(path, {
    method: "POST",
    signal,
    ...(payload ? { body: JSON.stringify(payload) } : {}),
  })
export const querySource = (id, query, signal) => post(`/api/v1/sources/${id}/query`, query, signal)
export const capabilities = (source) =>
  state.extensions.find((entry) => entry.id === source.extensionId)?.capabilities ?? []
export const route = (view, params = {}) => `#${view}?${new URLSearchParams(params)}`

export function link(label, href, className) {
  const result = node("a", label, className)
  result.href = href
  return result
}

export function heading(title, description) {
  const header = node("header", undefined, "mb-6")
  const h1 = node("h1", title)
  h1.tabIndex = -1
  header.append(h1)
  if (description) header.append(node("p", description, "text-light"))
  return header
}

export function raw(value) {
  const pre = node("pre")
  pre.append(node("code", JSON.stringify(value, null, 2)))
  return pre
}

export function disclosure(label, content) {
  const details = node("details")
  details.append(node("summary", label), content)
  return details
}

export function showDetail(title, content) {
  element("detail-title").textContent = title
  element("detail-content").replaceChildren(content)
  if (!element("detail").open) element("detail").showModal()
}

export function table(headers) {
  const wrap = node("div", undefined, "table")
  const result = node("table")
  const head = node("thead")
  const row = node("tr")
  for (const title of headers) {
    const cell = node("th", title)
    cell.scope = "col"
    row.append(cell)
  }
  head.append(row)
  const body = node("tbody")
  result.append(head, body)
  wrap.append(result)
  return { wrap, body }
}

/** Lazy authenticated images are fetched only as they approach the viewport. */
export function imageLoader(signal) {
  const objects = new Set()
  const observer = new IntersectionObserver(
    (entries) => {
      for (const entry of entries) {
        if (!entry.isIntersecting) continue
        observer.unobserve(entry.target)
        const image = entry.target
        if (!state.token) {
          image.src = image.dataset.url
          continue
        }
        api(image.dataset.url, { signal })
          .then((response) => response.blob())
          .then((blob) => {
            if (signal.aborted) return
            const url = URL.createObjectURL(blob)
            objects.add(url)
            image.src = url
          })
          .catch((error) => {
            if (error.name !== "AbortError") image.dispatchEvent(new Event("error"))
          })
      }
    },
    { rootMargin: "400px" },
  )
  signal.addEventListener(
    "abort",
    () => {
      observer.disconnect()
      for (const url of objects) URL.revokeObjectURL(url)
    },
    { once: true },
  )
  return (url, alt, className = "cover") => {
    const image = node("img", undefined, className)
    image.alt = alt
    image.loading = "lazy"
    image.decoding = "async"
    image.dataset.url = url
    image.addEventListener(
      "error",
      () => {
        image.hidden = true
        image.after(
          node(
            "p",
            className.includes("cover")
              ? "Cover unavailable"
              : `${alt} could not load. Refresh this view to retry.`,
            className.includes("cover") ? "cover p-4 text-light" : "text-light",
          ),
        )
      },
      { once: true },
    )
    observer.observe(image)
    return image
  }
}
