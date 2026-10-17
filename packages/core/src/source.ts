import { Schema } from "effect"
import { AssetReference } from "./asset.js"
import { Count, Text } from "./primitives.js"
import { PublicationMetadata } from "./publication.js"

/** Stable opaque identity within one source. Never infer identity from a title. */
export const SourceKey = Text
export const Cursor = Schema.NonEmptyString.check(Schema.isMaxLength(8192))

/** Cursors belong to the source; clients pass them back unchanged. */
export const Pagination = {
  cursor: Schema.optional(Cursor),
  limit: Schema.Int.check(Schema.isBetween({ minimum: 1, maximum: 100 })),
}

/** A work, independent of its translations, releases, and downloadable files. */
export const Series = Schema.Struct({
  key: SourceKey,
  title: Text,
  alternateTitles: Schema.Array(Text).check(Schema.isMaxLength(100)),
  status: Schema.Literals(["unknown", "ongoing", "completed", "hiatus", "cancelled"]),
  languages: Schema.Array(Text).check(Schema.isMaxLength(100)),
  metadata: PublicationMetadata,
  cover: Schema.optional(AssetReference),
})
export type Series = typeof Series.Type

/** A particular release. Different languages/groups must have different keys. */
export const Chapter = Schema.Struct({
  key: SourceKey,
  seriesKey: SourceKey,
  title: Text,
  number: Schema.optional(Text),
  volume: Schema.optional(Text),
  language: Schema.optional(Text),
  groups: Schema.Array(Text).check(Schema.isMaxLength(32)),
  publishedAt: Schema.optional(Count),
  pageCount: Schema.optional(Count),
})
export type Chapter = typeof Chapter.Type

/** Zero-based position in a chapter. Array order is the reading order. */
export const ChapterPage = Schema.Struct({
  index: Count,
  asset: AssetReference,
  mediaType: Schema.String.check(Schema.isPattern(/^image\/[a-z0-9.+-]+$/)),
  width: Schema.optional(Schema.Int.check(Schema.isGreaterThan(0))),
  height: Schema.optional(Schema.Int.check(Schema.isGreaterThan(0))),
})
export type ChapterPage = typeof ChapterPage.Type

export const SeriesPage = Schema.Struct({
  items: Schema.Array(Series).check(Schema.isMaxLength(100)),
  nextCursor: Schema.NullOr(Cursor),
})
export const ChapterList = Schema.Struct({
  items: Schema.Array(Chapter).check(Schema.isMaxLength(100)),
  nextCursor: Schema.NullOr(Cursor),
})
export const PageList = Schema.Struct({
  items: Schema.Array(ChapterPage).check(Schema.isMaxLength(100)),
  nextCursor: Schema.NullOr(Cursor),
})

/** Each operation is read-only and may be retried after a transient failure. */
export const SourceQuery = Schema.Union([
  Schema.Struct({
    operation: Schema.Literal("browse"),
    ...Pagination,
    sort: Schema.optional(Text),
    filters: Schema.Record(Text, Text),
  }),
  Schema.Struct({
    operation: Schema.Literal("search"),
    ...Pagination,
    text: Text,
    filters: Schema.Record(Text, Text),
  }),
  Schema.Struct({ operation: Schema.Literal("series"), seriesKey: SourceKey }),
  Schema.Struct({ operation: Schema.Literal("chapters"), ...Pagination, seriesKey: SourceKey }),
  Schema.Struct({ operation: Schema.Literal("pages"), ...Pagination, chapterKey: SourceKey }),
])
export type SourceQuery = typeof SourceQuery.Type

/** Internal source results may contain credentials in asset references. Never send them directly to a client. */
export const SourceResult = Schema.Union([
  Schema.Struct({ operation: Schema.Literal("browse"), data: SeriesPage }),
  Schema.Struct({ operation: Schema.Literal("search"), data: SeriesPage }),
  Schema.Struct({ operation: Schema.Literal("series"), data: Series }),
  Schema.Struct({ operation: Schema.Literal("chapters"), data: ChapterList }),
  Schema.Struct({ operation: Schema.Literal("pages"), data: PageList }),
])
export type SourceResult = typeof SourceResult.Type

export type QueryFor<K extends SourceQuery["operation"]> = Extract<SourceQuery, { operation: K }>
export type ResultFor<K extends SourceQuery["operation"]> = Extract<
  SourceResult,
  { operation: K }
>["data"]
