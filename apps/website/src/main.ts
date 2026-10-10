const dialog = document.querySelector<HTMLDialogElement>("#reader-dialog")!
const canvas = document.querySelector<HTMLDivElement>("#reader-canvas")!
const pageButton = document.querySelector<HTMLButtonElement>("#mode-page")!
const scrollButton = document.querySelector<HTMLButtonElement>("#mode-scroll")!
const note = document.querySelector<HTMLParagraphElement>("#mode-note")!

function setMode(scroll: boolean) {
  canvas.classList.toggle("scroll-mode", scroll)
  pageButton.setAttribute("aria-pressed", String(!scroll))
  scrollButton.setAttribute("aria-pressed", String(scroll))
  canvas.scrollTo({ top: 0, left: 0 })
  note.textContent = scroll
    ? "Take your time. Scroll to explore the artwork."
    : "The whole page, with room to breathe."
}

for (const id of ["open-reader", "open-reader-secondary", "open-reader-hero"]) {
  document.getElementById(id)!.addEventListener("click", () => {
    for (const image of dialog.querySelectorAll<HTMLImageElement>("img[data-src]")) {
      image.src = image.dataset.src!
      delete image.dataset.src
    }
    setMode(false)
    dialog.showModal()
  })
}
document.getElementById("close-reader")!.addEventListener("click", () => dialog.close())
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
