import { ExtensionError, type DiscoveryEvent } from "@chek/extensions"
import { Effect, Stream } from "effect"
import { opendir } from "node:fs/promises"
import { basename, join, relative } from "node:path"

const formats: Record<string, readonly [string, string]> = {
  epub: ["epub", "application/epub+zip"],
  pdf: ["pdf", "application/pdf"],
  cbz: ["cbz", "application/vnd.comicbook+zip"],
  cbr: ["cbr", "application/vnd.comicbook-rar"],
  cb7: ["cb7", "application/x-cb7"],
  cbt: ["cbt", "application/x-cbt"],
  mobi: ["mobi", "application/x-mobipocket-ebook"],
  prc: ["mobi", "application/x-mobipocket-ebook"],
  azw3: ["azw3", "application/vnd.amazon.ebook"],
  fb2: ["fb2", "application/x-fictionbook+xml"],
  fbz: ["fb2.zip", "application/x-zip-compressed-fb2"],
  txt: ["txt", "text/plain"],
  md: ["markdown", "text/markdown"],
  markdown: ["markdown", "text/markdown"],
  html: ["html", "text/html"],
  xhtml: ["html", "application/xhtml+xml"],
  rtf: ["rtf", "application/rtf"],
  docx: ["docx", "application/vnd.openxmlformats-officedocument.wordprocessingml.document"],
  odt: ["odt", "application/vnd.oasis.opendocument.text"],
  djvu: ["djvu", "image/vnd.djvu"],
  doc: ["doc", "application/msword"],
  pdb: ["pdb", "application/vnd.palm"],
  azw: ["azw", "application/vnd.amazon.ebook"],
  zip: ["zip", "application/zip"],
  rar: ["rar", "application/vnd.rar"],
  "7z": ["7z", "application/x-7z-compressed"],
  tar: ["tar", "application/x-tar"],
  jpg: ["jpeg", "image/jpeg"],
  jpeg: ["jpeg", "image/jpeg"],
  png: ["png", "image/png"],
  webp: ["webp", "image/webp"],
  avif: ["avif", "image/avif"],
  gif: ["gif", "image/gif"],
  svg: ["svg", "image/svg+xml"],
}

async function* walk(
  root: string,
  directory: string,
  signal: AbortSignal,
  depth = 0,
): AsyncGenerator<DiscoveryEvent> {
  if (depth > 64) {
    yield {
      type: "warning",
      code: "directory-depth",
      message: "Skipped a directory deeper than 64 levels",
    }
    return
  }
  let entries
  try {
    entries = await opendir(directory)
  } catch {
    if (depth === 0) throw new Error("Cannot read source folder")
    yield {
      type: "warning",
      code: "directory-unreadable",
      message: "A subdirectory could not be read",
    }
    return
  }
  for await (const entry of entries) {
    signal.throwIfAborted()
    if (entry.name.startsWith(".") || entry.isSymbolicLink()) continue
    const path = join(directory, entry.name)
    if (entry.isDirectory()) {
      yield* walk(root, path, signal, depth + 1)
      continue
    }
    if (!entry.isFile()) continue
    const suffix = entry.name.toLowerCase().endsWith(".fb2.zip")
      ? "fbz"
      : entry.name.split(".").at(-1)?.toLowerCase()
    const format = suffix ? formats[suffix] : undefined
    if (!format) continue
    yield {
      type: "publication",
      publication: {
        sourceKey: relative(root, path),
        title: basename(entry.name).replace(/\.(?:fb2\.zip|[^.]+)$/i, ""),
        format: format[0],
        mediaType: format[1],
        authors: [],
        asset: { kind: "file", path },
      },
    }
  }
}

export const discoverFiles = (root: string) =>
  Stream.unwrap(
    Effect.gen(function* () {
      const controller = yield* Effect.acquireRelease(
        Effect.sync(() => new AbortController()),
        (value) => Effect.sync(() => value.abort()),
      )

      return Stream.fromAsyncIterable(
        walk(root, root, controller.signal),
        () =>
          new ExtensionError({
            code: "directory-unreadable",
            message: "Cannot read source folder",
            retryable: true,
          }),
      )
    }),
  )
