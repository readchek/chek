import { Schema } from "effect"

// Decode the fields we consume; Komga can add response fields without breaking an extension.
const Author = Schema.Struct({ name: Schema.String, role: Schema.String })
export const KomgaSeries = Schema.Struct({
  id: Schema.NonEmptyString,
  libraryId: Schema.String,
  name: Schema.String,
  metadata: Schema.Struct({
    title: Schema.String,
    status: Schema.String,
    language: Schema.String,
    summary: Schema.String,
    publisher: Schema.String,
    readingDirection: Schema.String,
    genres: Schema.Array(Schema.String),
    tags: Schema.Array(Schema.String),
    alternateTitles: Schema.Array(Schema.Struct({ title: Schema.String })),
  }),
  booksMetadata: Schema.Struct({ authors: Schema.Array(Author), summary: Schema.String }),
})
export type KomgaSeries = typeof KomgaSeries.Type

export const KomgaBook = Schema.Struct({
  id: Schema.NonEmptyString,
  seriesId: Schema.NonEmptyString,
  libraryId: Schema.String,
  name: Schema.String,
  url: Schema.String,
  seriesTitle: Schema.String,
  media: Schema.Struct({
    status: Schema.String,
    mediaType: Schema.String,
    mediaProfile: Schema.String,
    pagesCount: Schema.Int,
    epubDivinaCompatible: Schema.Boolean,
  }),
  metadata: Schema.Struct({
    title: Schema.String,
    number: Schema.String,
    numberSort: Schema.Number,
    summary: Schema.String,
    isbn: Schema.String,
    authors: Schema.Array(Author),
    tags: Schema.Array(Schema.String),
    releaseDate: Schema.optional(Schema.NullOr(Schema.String)),
  }),
})
export type KomgaBook = typeof KomgaBook.Type

export const KomgaPage = Schema.Struct({
  number: Schema.Int.check(Schema.isGreaterThan(0)),
  mediaType: Schema.String,
  width: Schema.optional(Schema.NullOr(Schema.Int)),
  height: Schema.optional(Schema.NullOr(Schema.Int)),
})
export const KomgaUser = Schema.Struct({ roles: Schema.Array(Schema.String) })

export const paged = <S extends Schema.Top>(item: S) =>
  Schema.Struct({
    content: Schema.Array(item),
    last: Schema.Boolean,
    number: Schema.Int.check(Schema.isGreaterThanOrEqualTo(0)),
    size: Schema.Int.check(Schema.isGreaterThan(0)),
  })
