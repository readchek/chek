const dialog = document.querySelector<HTMLDialogElement>("#reader-dialog")!
const canvas = document.querySelector<HTMLDivElement>("#reader-canvas")!
const pageButton = document.querySelector<HTMLButtonElement>("#mode-page")!
const scrollButton = document.querySelector<HTMLButtonElement>("#mode-scroll")!
const note = document.querySelector<HTMLParagraphElement>("#mode-note")!
const images = Array.from(dialog.querySelectorAll<HTMLImageElement>("img[data-src]"))
let artworkState: "idle" | "loading" | "ready" | "error" = "idle"
let trigger: HTMLElement | null = null

function updateNote() {
  canvas.setAttribute("aria-busy", String(artworkState === "loading"))
  note.textContent =
    artworkState === "loading"
      ? "Loading the detailed page. You can keep reading the preview."
      : artworkState === "error"
        ? "Showing the preview. Close and reopen to retry the detailed page."
        : canvas.classList.contains("scroll-mode")
          ? "Take your time. Scroll to explore the artwork."
          : "The whole page, with room to breathe."
}

async function loadArtwork() {
  if (artworkState === "loading" || artworkState === "ready") return
  artworkState = "loading"
  const preview = document.querySelector<HTMLImageElement>(".phone-page img")!
  for (const image of images) image.src = preview.currentSrc || preview.src
  updateNote()
  try {
    const detailed = new Image()
    detailed.src = images[0]!.dataset.src!
    await detailed.decode()
    for (const image of images) image.src = detailed.src
    artworkState = "ready"
  } catch {
    artworkState = "error"
  }
  updateNote()
}

function setMode(scroll: boolean) {
  canvas.classList.toggle("scroll-mode", scroll)
  pageButton.setAttribute("aria-pressed", String(!scroll))
  scrollButton.setAttribute("aria-pressed", String(scroll))
  canvas.scrollTo({ top: 0, left: 0, behavior: "instant" })
  updateNote()
}

for (const id of ["open-reader", "open-reader-secondary", "open-reader-hero"]) {
  document.getElementById(id)!.addEventListener("click", (event) => {
    trigger = event.currentTarget as HTMLElement
    setMode(false)
    dialog.showModal()
    void loadArtwork()
  })
}
document.getElementById("close-reader")!.addEventListener("click", () => dialog.close())
dialog.addEventListener("close", () => trigger?.focus({ preventScroll: true }))
pageButton.addEventListener("click", () => setMode(false))
scrollButton.addEventListener("click", () => setMode(true))
dialog.addEventListener("click", (event) => {
  const bounds = dialog.getBoundingClientRect()
  if (
    event.target === dialog &&
    (event.clientX < bounds.left ||
      event.clientX > bounds.right ||
      event.clientY < bounds.top ||
      event.clientY > bounds.bottom)
  )
    dialog.close()
})
