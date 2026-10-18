import { element, heading, json, link, node, notice, route, state, capabilities } from "./ui.js"
import { browse, library, publication, reader, series } from "./library.js"
import { access, extensions, jobs, sources } from "./admin.js"

const views = { browse, library, publication, reader, series, access, extensions, jobs, sources }
let controller

async function refresh(signal) {
  const [installed, connected] = await Promise.all([
    json("/api/v1/extensions", { signal }),
    json("/api/v1/sources", { signal }),
  ])
  signal?.throwIfAborted()
  state.extensions = installed
  state.sources = connected
  const nav = element("source-nav")
  nav.replaceChildren()
  for (const source of connected) {
    const caps = capabilities(source)
    if (!caps.includes("browse") && !caps.includes("search")) continue
    const item = node("li")
    item.append(link(source.name, route("browse", { source: source.id })))
    nav.append(item)
  }
  element("connection").textContent = "Connected"
}

async function navigate() {
  controller?.abort()
  if (element("detail").open) element("detail").close()
  controller = new AbortController()
  const { signal } = controller
  const [name = "library", query = ""] = location.hash.slice(1).split("?")
  const params = new URLSearchParams(query)
  const view = element("view")
  const content = node("section")
  view.replaceChildren(content)
  view.setAttribute("aria-busy", "true")
  notice("")
  element("main").scrollTop = 0
  document.body.removeAttribute("data-sidebar-open")
  for (const item of document.querySelectorAll("[data-sidebar] a")) {
    const selected =
      item.hash === `#${name}` ||
      (params.get("source") && item.hash === route("browse", { source: params.get("source") }))
    if (selected) item.setAttribute("aria-current", "page")
    else item.removeAttribute("aria-current")
  }
  const loading = node("p", "Loading…", "text-light")
  content.append(loading)
  try {
    const render = views[name || "library"]
    if (!render) throw new Error("Unknown view. Choose a destination from the sidebar.")
    await render(content, params, signal, refresh)
  } catch (error) {
    if (signal.aborted) return
    const message = node("p", error.message)
    message.setAttribute("role", "alert")
    content.append(message, link("Retry", location.hash || "#library", "button outline"))
    content.lastChild.addEventListener("click", (event) => {
      event.preventDefault()
      navigate()
    })
  } finally {
    loading.remove()
    if (!signal.aborted) {
      view.setAttribute("aria-busy", "false")
      const title = content.querySelector("h1")
      document.title = `${title?.textContent || "Inspector"} · Chek`
      title?.focus({ preventScroll: true })
    }
  }
}

// Oat owns the sidebar interaction; keep closed mobile navigation out of the tab order.
const mobile = matchMedia("(max-width: 768px)")
const toggle = document.querySelector("[data-sidebar-toggle]")
function syncSidebar() {
  const open = document.body.hasAttribute("data-sidebar-open")
  toggle.setAttribute("aria-expanded", String(open))
  element("sidebar").inert = mobile.matches && !open
}
new MutationObserver(syncSidebar).observe(document.body, {
  attributes: true,
  attributeFilter: ["data-sidebar-open"],
})
mobile.addEventListener("change", syncSidebar)
document.addEventListener("keydown", (event) => {
  if (event.key === "Escape" && document.body.hasAttribute("data-sidebar-open")) {
    document.body.removeAttribute("data-sidebar-open")
    toggle.focus()
  }
})
syncSidebar()

window.addEventListener("hashchange", navigate)
try {
  await refresh()
  await navigate()
} catch (error) {
  element("connection").textContent = "Connection failed"
  element("view").setAttribute("aria-busy", "false")
  element("view").replaceChildren(
    heading("Cannot connect to Chek"),
    link("Set server access", "#access", "button"),
  )
  notice(error.message, true)
}
