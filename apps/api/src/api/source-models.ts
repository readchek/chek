import { ChapterList, ChapterPage, Cursor, Series } from "@chek/core"
import { Schema } from "effect"

/** Browser-facing models contain server URLs, never upstream headers or filesystem paths. */
export const PublicSeries = Schema.Struct({
  ...Series.fields,
  cover: Schema.optional(Schema.String),
})
const PublicSeriesPage = Schema.Struct({
  items: Schema.Array(PublicSeries),
  nextCursor: Schema.NullOr(Cursor),
})
const PublicPages = Schema.Struct({
  items: Schema.Array(Schema.Struct({ ...ChapterPage.fields, asset: Schema.String })),
  nextCursor: Schema.NullOr(Cursor),
})
export const PublicSourceResult = Schema.Union([
  Schema.Struct({ operation: Schema.Literal("browse"), data: PublicSeriesPage }),
  Schema.Struct({ operation: Schema.Literal("search"), data: PublicSeriesPage }),
  Schema.Struct({ operation: Schema.Literal("series"), data: PublicSeries }),
  Schema.Struct({ operation: Schema.Literal("chapters"), data: ChapterList }),
  Schema.Struct({ operation: Schema.Literal("pages"), data: PublicPages }),
])
