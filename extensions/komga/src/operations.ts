import { Effect, Option, Schema, Stream } from "effect"
import type { DiscoveryEvent, QueryFor } from "@chek/extensions"
import { asset, chapterModel, publicationModel, seriesModel } from "./mapping.js"
import {
  checkLibrary,
  cursorPage,
  endpoint,
  equal,
  komgaError,
  nextCursor,
  request,
  visible,
  type Config,
} from "./client.js"
import { KomgaBook, KomgaPage, KomgaSeries, KomgaUser, paged } from "./models.js"

const scope = (config: Config) =>
  config.libraryId ? [visible, equal("libraryId", config.libraryId)] : [visible]

/** Supported filters map to Komga's typed search conditions, never raw query expressions. */
export const listSeries = Effect.fn("komga.series.list")(function* (
  config: Config,
  query: QueryFor<"browse"> | QueryFor<"search">,
) {
  const page = yield* cursorPage(query.cursor, query.limit)
  const sorts: Record<string, string> = {
    title: "metadata.titleSort,asc",
    updated: "lastModified,desc",
    added: "created,desc",
  }
  const sort = query.operation === "browse" ? (query.sort ?? "title") : "title"
  if (!sorts[sort]) return yield* komgaError("komga-sort", "Supported sorts: title, updated, added")
  const conditions = scope(config)
  const fields: Record<string, string> = {
    library: "libraryId",
    collection: "collectionId",
    language: "language",
    genre: "genre",
    tag: "tag",
    publisher: "publisher",
    status: "seriesStatus",
  }
  for (const [name, value] of Object.entries(query.filters)) {
    if (!fields[name]) return yield* komgaError("komga-filter", `Unsupported filter: ${name}`)
    conditions.push(equal(fields[name], value))
  }
  const params = new URLSearchParams({ page: String(page), size: String(query.limit) })
  params.append("sort", sorts[sort]!)
  params.append("sort", "id,asc")
  const result = yield* request(config, `v1/series/list?${params}`, paged(KomgaSeries), {
    condition: { allOf: conditions },
    ...(query.operation === "search" ? { fullTextSearch: query.text } : {}),
  })
  return {
    items: result.content.map((series) => seriesModel(config, series)),
    nextCursor: nextCursor(result, query.limit),
  }
})

export const getSeries = Effect.fn("komga.series.get")(function* (config: Config, key: string) {
  const series = yield* request(config, `v1/series/${encodeURIComponent(key)}`, KomgaSeries)
  yield* checkLibrary(config, series.libraryId)
  return series
})

export const chapters = Effect.fn("komga.chapters")(function* (
  config: Config,
  query: QueryFor<"chapters">,
) {
  const series = yield* getSeries(config, query.seriesKey)
  const page = yield* cursorPage(query.cursor, query.limit)
  const result = yield* request(
    config,
    `v1/books/list?page=${page}&size=${query.limit}&sort=metadata.numberSort,asc&sort=id,asc`,
    paged(KomgaBook),
    {
      condition: { allOf: [...scope(config), equal("seriesId", query.seriesKey)] },
    },
  )
  return {
    items: result.content.map((book) => chapterModel(book, series)),
    nextCursor: nextCursor(result, query.limit),
  }
})

export const pages = Effect.fn("komga.pages")(function* (config: Config, query: QueryFor<"pages">) {
  const book = yield* request(config, `v1/books/${encodeURIComponent(query.chapterKey)}`, KomgaBook)
  yield* checkLibrary(config, book.libraryId)
  if (book.media.status !== "READY")
    return yield* komgaError(
      "komga-analysis",
      book.media.status === "ERROR"
        ? "Komga could not analyze this book. Check its media status in Komga."
        : "Komga has not finished analyzing this book",
      book.media.status === "UNKNOWN" || book.media.status === "OUTDATED",
    )
  if (book.media.mediaType === "application/epub+zip" && !book.media.epubDivinaCompatible)
    return yield* komgaError(
      "komga-document",
      "This EPUB needs a document reader. Import its original through the source scan.",
    )
  const position = yield* cursorPage(query.cursor, query.limit)
  const entries = yield* request(
    config,
    `v1/books/${encodeURIComponent(book.id)}/pages`,
    Schema.Array(KomgaPage),
  )
  const start = position * query.limit
  if (start > entries.length)
    return yield* komgaError("komga-cursor", "Page cursor is beyond the end of this chapter")
  return {
    items: entries.slice(start, start + query.limit).map((page) => {
      const native = /^image\/(jpeg|png|gif|webp|avif)$/.test(page.mediaType)
      return {
        index: page.number - 1,
        asset: asset(
          config,
          `v1/books/${encodeURIComponent(book.id)}/pages/${page.number}?zero_based=false&contentNegotiation=false${native ? "" : "&convert=png"}`,
        ),
        mediaType: native ? page.mediaType : "image/png",
        ...(page.width && page.width > 0 ? { width: page.width } : {}),
        ...(page.height && page.height > 0 ? { height: page.height } : {}),
      }
    }),
    nextCursor:
      start + query.limit < entries.length
        ? JSON.stringify({ page: position + 1, size: query.limit })
        : null,
  }
})

/** Enumerate original files with backpressure. Full scans avoid stale offset checkpoints after library edits. */
export const discover = (config: Config) =>
  Stream.unwrap(
    Effect.gen(function* () {
      const user = yield* request(config, "v2/users/me", KomgaUser)
      if (!user.roles.includes("FILE_DOWNLOAD") && !user.roles.includes("ADMIN"))
        return yield* komgaError(
          "komga-download-permission",
          "Enable file download permission in Komga to import originals",
        )
      return Stream.paginate(0, (page) =>
        request(config, `v1/books/list?page=${page}&size=100&sort=id,asc`, paged(KomgaBook), {
          condition: { allOf: scope(config) },
        }).pipe(
          Effect.map(
            (result) =>
              [
                result.content.map((book): DiscoveryEvent => {
                  if (book.media.status !== "READY")
                    return {
                      type: "warning",
                      code: "komga-analysis",
                      message: `Skipped book ${book.id}: Komga media status is ${book.media.status}`,
                    }
                  const publication = publicationModel(config, book)
                  return publication
                    ? { type: "publication", publication }
                    : {
                        type: "warning",
                        code: "komga-format",
                        message: `Skipped book ${book.id}: unsupported media type`,
                      }
                }),
                result.last ? Option.none<number>() : Option.some(page + 1),
              ] as const,
          ),
        ),
      )
    }),
  )

export const validateAccess = Effect.fn("komga.validate")(function* (config: Config) {
  yield* request(config, "v2/users/me", KomgaUser)
  if (config.libraryId)
    yield* request(
      config,
      `v1/libraries/${encodeURIComponent(config.libraryId)}`,
      Schema.Struct({ id: Schema.String }),
    )
  yield* Effect.logDebug("Komga access verified", { origin: new URL(endpoint(config, "")).origin })
})
