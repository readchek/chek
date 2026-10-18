import type { Chapter, PublicationInput, Series } from "@chek/extensions"
import { authentication, endpoint, type Config } from "./client.js"
import type { KomgaBook, KomgaSeries } from "./models.js"

export const asset = (config: Config, path: string) => ({
  kind: "http" as const,
  url: endpoint(config, path),
  headers: authentication(config),
})

export function seriesModel(config: Config, source: KomgaSeries): Series {
  const metadata = source.metadata
  const status = {
    ENDED: "completed",
    ONGOING: "ongoing",
    ABANDONED: "cancelled",
    HIATUS: "hiatus",
  } as const
  return {
    key: source.id,
    title: metadata.title || source.name,
    alternateTitles: metadata.alternateTitles.map((title) => title.title).filter(Boolean),
    status: status[metadata.status as keyof typeof status] ?? "unknown",
    languages: metadata.language ? [metadata.language] : [],
    cover: asset(config, `v1/series/${encodeURIComponent(source.id)}/thumbnail`),
    metadata: {
      ...(metadata.summary || source.booksMetadata.summary
        ? { description: metadata.summary || source.booksMetadata.summary }
        : {}),
      ...(metadata.publisher ? { publisher: metadata.publisher } : {}),
      contributors: source.booksMetadata.authors.map((author) => ({
        name: author.name,
        roles: [author.role],
      })),
      subjects: [...new Set([...metadata.genres, ...metadata.tags])],
      readingProgression:
        metadata.readingDirection === "RIGHT_TO_LEFT"
          ? "rtl"
          : metadata.readingDirection === "LEFT_TO_RIGHT"
            ? "ltr"
            : "auto",
    },
  }
}

/** Komga books are releases; keep their IDs and displayed issue numbers, including decimals/specials. */
export function chapterModel(book: KomgaBook, series: KomgaSeries): Chapter {
  const date = book.metadata.releaseDate ? Date.parse(book.metadata.releaseDate) : NaN
  return {
    key: book.id,
    seriesKey: book.seriesId,
    title: book.metadata.title || book.name,
    ...(book.metadata.number ? { number: book.metadata.number } : {}),
    ...(series.metadata.language ? { language: series.metadata.language } : {}),
    ...(Number.isFinite(date) && date >= 0 ? { publishedAt: date } : {}),
    groups: [],
    pageCount: Math.max(0, book.media.pagesCount),
  }
}

/** Downloads preserve EPUB/PDF/archive originals; image paging is a separate capability. */
export function publicationModel(config: Config, book: KomgaBook): PublicationInput | undefined {
  const mediaType = book.media.mediaType
  const formats: Record<string, PublicationInput["format"]> = {
    "application/epub+zip": "epub",
    "application/pdf": "pdf",
    "application/zip": "cbz",
    "application/x-cbz": "cbz",
    "application/vnd.comicbook+zip": "cbz",
    "application/x-rar-compressed": "cbr",
    "application/vnd.rar": "cbr",
    "application/x-cbr": "cbr",
    "application/vnd.comicbook-rar": "cbr",
  }
  const format = formats[mediaType]
  if (!format) return undefined
  return {
    sourceKey: book.id,
    title: book.metadata.title || book.name,
    format,
    mediaType,
    authors: book.metadata.authors.map((author) => author.name),
    metadata: {
      ...(book.metadata.summary ? { description: book.metadata.summary } : {}),
      ...(book.metadata.isbn ? { identifier: book.metadata.isbn } : {}),
      series: { name: book.seriesTitle },
      subjects: book.metadata.tags,
      contributors: book.metadata.authors.map((author) => ({
        name: author.name,
        roles: [author.role],
      })),
    },
    asset: asset(config, `v1/books/${encodeURIComponent(book.id)}/file`),
  }
}
