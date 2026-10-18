import {
  action,
  api,
  capabilities,
  disclosure,
  heading,
  imageLoader,
  json,
  link,
  node,
  querySource,
  raw,
  route,
  state,
  table,
} from "./ui.js"

function searchForm(label, text, destination) {
  const form = node("form", undefined, "search-bar")
  form.setAttribute("role", "search")
  const input = node("input")
  input.type = "search"
  input.value = text
  input.placeholder = label
  input.setAttribute("aria-label", label)
  input.maxLength = 256
  form.append(input, node("button", "Search"))
  if (text) form.append(link("Clear", destination(""), "button outline"))
  form.addEventListener("submit", (event) => {
    event.preventDefault()
    location.hash = destination(input.value.trim())
  })
  return form
}

function sourceFor(params) {
  const source = state.sources.find((entry) => entry.id === params.get("source"))
  if (!source) throw new Error("Source not found. Choose a source from the sidebar.")
  return source
}

/** Append one page at a time; controls remain retryable after a failed request. */
async function paginate(container, signal, load, render, label, empty) {
  let cursor
  let count = 0
  const status = node("p", "Loading…", "text-light")
  status.setAttribute("role", "status")
  const more = action(label, async () => {
    const result = await load(cursor)
    signal.throwIfAborted()
    render(result.items)
    count += result.items.length
    cursor = result.nextCursor
    more.hidden = !cursor
    status.textContent = count ? `${count} loaded${cursor ? " · more available" : ""}` : empty
  })
  more.hidden = true
  container.append(status, more)
  let first
  try {
    first = await load()
  } catch (error) {
    status.remove()
    throw error
  }
  signal.throwIfAborted()
  render(first.items)
  count = first.items.length
  cursor = first.nextCursor
  more.hidden = !cursor
  status.textContent = count ? `${count} loaded${cursor ? " · more available" : ""}` : empty
}

export async function browse(container, params, signal) {
  const source = sourceFor(params)
  const text = params.get("q") || ""
  const caps = capabilities(source)
  container.append(
    heading(source.name, "Browse series, open a book, and read pages from this source."),
  )
  if (caps.includes("search"))
    container.append(
      searchForm("Search this source", text, (q) => route("browse", { source: source.id, q })),
    )
  const operation = text ? "search" : "browse"
  if (!caps.includes(operation)) {
    container.append(
      node(
        "p",
        operation === "browse"
          ? "Enter a search to find books in this source."
          : "This source does not support search.",
      ),
    )
    return
  }
  const image = imageLoader(signal)
  const shelf = node("ul", undefined, "shelf")
  container.append(shelf)
  await paginate(
    container,
    signal,
    async (cursor) =>
      (
        await querySource(
          source.id,
          {
            operation,
            limit: 24,
            filters: {},
            ...(text ? { text } : {}),
            ...(cursor ? { cursor } : {}),
          },
          signal,
        )
      ).data,
    (items) => {
      for (const series of items) {
        const item = node("li")
        const card = link("", route("series", { source: source.id, key: series.key }))
        if (series.cover) card.append(image(series.cover, `Cover of ${series.title}`))
        else card.append(node("div", "No cover", "cover p-4 text-light"))
        card.append(
          node("h3", series.title),
          node("small", [series.status, ...series.languages].join(" · "), "text-light"),
        )
        item.append(card)
        shelf.append(item)
      }
    },
    "Load more series",
    text ? "No series match your search." : "This source has no series yet.",
  )
}

export async function series(container, params, signal) {
  const source = sourceFor(params)
  const key = params.get("key")
  const caps = capabilities(source)
  container.append(link(`← ${source.name}`, route("browse", { source: source.id })))
  const result = caps.includes("series")
    ? (await querySource(source.id, { operation: "series", seriesKey: key }, signal)).data
    : { key, title: key, metadata: {} }
  signal.throwIfAborted()
  const row = node("div", undefined, "row mt-6 mb-6")
  const details = node("div", undefined, result.cover ? "col-9" : "col-12")
  if (result.cover) {
    const cover = node("div", undefined, "col-3")
    cover.append(
      imageLoader(signal)(result.cover, `Cover of ${result.title}`, "cover series-cover"),
    )
    row.append(cover)
  }
  details.append(
    heading(result.title, [result.status, ...(result.languages || [])].filter(Boolean).join(" · ")),
  )
  if (result.metadata.description) details.append(node("p", result.metadata.description))
  if (result.metadata.contributors?.length)
    details.append(node("p", result.metadata.contributors.map((entry) => entry.name).join(", ")))
  if (result.metadata.publisher) details.append(node("p", result.metadata.publisher, "text-light"))
  details.append(disclosure("Series data", raw(result)))
  row.append(details)
  container.append(row, node("h2", "Books & chapters"))
  if (!caps.includes("chapters")) {
    container.append(node("p", "This extension does not provide chapters."))
    return
  }
  const { wrap, body } = table(["Title", "Number", "Pages", "Language", ""])
  container.append(wrap)
  await paginate(
    container,
    signal,
    async (cursor) =>
      (
        await querySource(
          source.id,
          {
            operation: "chapters",
            seriesKey: key,
            limit: 30,
            ...(cursor ? { cursor } : {}),
          },
          signal,
        )
      ).data,
    (items) => {
      for (const chapter of items) {
        const tr = node("tr")
        for (const value of [
          chapter.title,
          chapter.number ?? "—",
          chapter.pageCount ?? "—",
          chapter.language || "—",
        ])
          tr.append(node("td", String(value)))
        const cell = node("td")
        if (caps.includes("pages"))
          cell.append(
            link(
              "Read",
              route("reader", {
                source: source.id,
                series: key,
                key: chapter.key,
                title: chapter.title,
              }),
              "button small",
            ),
          )
        tr.append(cell)
        body.append(tr)
      }
    },
    "Load more books",
    "No books in this series.",
  )
}

export async function reader(container, params, signal) {
  const source = sourceFor(params)
  const title = params.get("title") || "Reader"
  container.append(
    link("← Back to series", route("series", { source: source.id, key: params.get("series") })),
    heading(title, source.name),
  )
  const toolbar = node("div", undefined, "hstack reader-toolbar mb-4")
  const label = node("label", "Page width")
  const width = node("select")
  width.id = "reader-width"
  label.htmlFor = width.id
  for (const [value, text] of [
    ["comfortable", "Comfortable"],
    ["wide", "Full width"],
  ]) {
    const option = node("option", text)
    option.value = value
    width.append(option)
  }
  const pages = node("div", undefined, "reader")
  width.value = localStorage.getItem("chek-reader-width") || "comfortable"
  pages.dataset.width = width.value
  width.addEventListener("change", () => {
    pages.dataset.width = width.value
    localStorage.setItem("chek-reader-width", width.value)
  })
  toolbar.append(label, width)
  container.append(toolbar, pages)
  const image = imageLoader(signal)
  await paginate(
    container,
    signal,
    async (cursor) =>
      (
        await querySource(
          source.id,
          {
            operation: "pages",
            chapterKey: params.get("key"),
            limit: 10,
            ...(cursor ? { cursor } : {}),
          },
          signal,
        )
      ).data,
    (items) => {
      for (const page of items) {
        const figure = node("figure")
        const img = image(page.asset, `${title}, page ${page.index + 1}`, "")
        if (page.width && page.height) {
          img.width = page.width
          img.height = page.height
        }
        figure.append(img, node("figcaption", `Page ${page.index + 1}`))
        pages.append(figure)
      }
    },
    "Load next pages",
    "This book has no image pages.",
  )
}

export async function library(container, params, signal) {
  const text = params.get("q") || ""
  container.append(
    heading(
      "Imported books",
      "Original files stored by Chek. Browse a source in the sidebar to read its available pages.",
    ),
    searchForm("Search imported titles", text, (q) => route("library", { q })),
  )
  const { wrap, body } = table(["Title", "Format", "Size", "Status", ""])
  container.append(wrap)
  await paginate(
    container,
    signal,
    (cursor) =>
      json(
        `/api/v1/publications?${new URLSearchParams({ search: text, limit: "30", ...(cursor ? { after: cursor } : {}) })}`,
        { signal },
      ),
    (items) => {
      for (const book of items) {
        const row = node("tr")
        for (const value of [
          book.title,
          book.format.toUpperCase(),
          `${(book.byteLength / 1048576).toFixed(1)} MiB`,
          book.status,
        ])
          row.append(node("td", value))
        const cell = node("td")
        cell.append(link("Details", route("publication", { id: book.id })))
        row.append(cell)
        body.append(row)
      }
    },
    "Load more books",
    "No imported books match. Add a source and start an import from Manage sources.",
  )
}

export async function publication(container, params, signal) {
  const book = await json(`/api/v1/publications/${encodeURIComponent(params.get("id"))}`, {
    signal,
  })
  signal.throwIfAborted()
  container.append(
    link("← Imported books", "#library"),
    heading(
      book.title,
      `${book.format.toUpperCase()} · ${(book.byteLength / 1048576).toFixed(1)} MiB`,
    ),
  )
  const url = `/api/v1/assets/${book.assetHash}`
  const download = link("Download original", url, "button")
  download.download = `${book.title}.${book.format}`
  if (!state.token) container.append(download)
  else
    container.append(
      node("p", "Download with an Authorization header using the original asset URL:"),
      node("code", `${location.origin}${url}`),
    )
  const check = node("p")
  container.append(
    action("Check byte-range delivery", async () => {
      const response = await api(url, { signal, headers: { Range: "bytes=0-63" } })
      const bytes = await response.arrayBuffer()
      check.textContent = `HTTP ${response.status} · ${response.headers.get("Content-Range")} · ${bytes.byteLength} bytes`
    }),
    check,
    disclosure("Publication data", raw(book)),
  )
}
