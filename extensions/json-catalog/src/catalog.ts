import {
  Chapter,
  ChapterPage,
  ExtensionError,
  Series,
  SourceHttp,
  SourceState,
} from "@chek/extensions"
import { Effect, Schema } from "effect"

/** The reference catalog uses core models directly; provider-specific extensions map their API here. */
export const Catalog = Schema.Struct({
  series: Schema.Array(
    Schema.Struct({
      ...Series.fields,
      chapters: Schema.Array(
        Schema.Struct({
          ...Chapter.fields,
          pages: Schema.Array(ChapterPage).check(Schema.isMaxLength(10000)),
        }),
      ).check(Schema.isMaxLength(10000)),
    }),
  ).check(Schema.isMaxLength(10000)),
})
export const Config = Schema.Struct({
  url: Schema.NonEmptyString,
  token: Schema.optional(Schema.String),
})
export type Config = typeof Config.Type

export const sourceError = (code: string, message: string) =>
  new ExtensionError({ code, message, retryable: false })

/** A short persistent cache survives process restarts; sessions and cached metadata remain source-scoped. */
export const loadCatalog = Effect.fn("catalog.load")(function* (config: Config) {
  const state = yield* SourceState
  const cached = yield* state.get("catalog")
  const Cached = Schema.Struct({ at: Schema.Number, catalog: Catalog })
  const decoded = yield* Schema.decodeUnknownEffect(Cached)(cached).pipe(
    Effect.orElseSucceed(() => undefined),
  )
  if (decoded && decoded.at > Date.now() - 30_000) return decoded.catalog

  const http = yield* SourceHttp
  const response = yield* http.request({
    url: config.url,
    headers: config.token ? { authorization: `Bearer ${config.token}` } : undefined,
  })
  if (response.status !== 200)
    return yield* sourceError(
      `catalog-http-${response.status}`,
      "Catalog access failed; check its URL and credentials",
    )
  const catalog = yield* Schema.decodeUnknownEffect(Schema.fromJsonString(Catalog))(
    response.text,
  ).pipe(
    Effect.mapError(() =>
      sourceError("catalog-invalid", "Catalog does not match the core series/chapter/page models"),
    ),
  )
  const seriesKeys = new Set<string>()
  const chapterKeys = new Set<string>()
  for (const series of catalog.series) {
    if (seriesKeys.has(series.key))
      return yield* sourceError("catalog-identity", "Series keys must be unique")
    seriesKeys.add(series.key)
    for (const chapter of series.chapters) {
      if (chapterKeys.has(chapter.key) || chapter.seriesKey !== series.key)
        return yield* sourceError(
          "catalog-identity",
          "Chapter keys must be source-unique and belong to their series",
        )
      chapterKeys.add(chapter.key)
      if (
        chapter.pages.some(
          (page, index, pages) => index > 0 && page.index <= pages[index - 1]!.index,
        )
      )
        return yield* sourceError(
          "catalog-page-order",
          "Page indices must be unique and increasing",
        )
    }
  }
  // State values are bounded to 64 KiB. Large catalogs use the host's query cache instead.
  if (response.byteLength < 24000) yield* state.set("catalog", { at: Date.now(), catalog })
  return catalog
})

/** Offset cursors are intentionally private to this extension; callers must not interpret them. */
export const paginate = <A>(items: readonly A[], query: { limit: number; cursor?: string }) =>
  Effect.gen(function* () {
    const offset = query.cursor === undefined ? 0 : Number(query.cursor)
    if (!Number.isSafeInteger(offset) || offset < 0 || offset > items.length)
      return yield* sourceError("cursor-invalid", "Catalog cursor is invalid; restart browsing")
    const end = offset + query.limit
    return { items: items.slice(offset, end), nextCursor: end < items.length ? String(end) : null }
  })
