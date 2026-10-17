import { defineExtension, ExtensionManifest, type Series, type Chapter } from "@chek/extensions"
import { Effect, Schema } from "effect"
import manifest from "../extension.json" with { type: "json" }
import { Config, loadCatalog, paginate, sourceError } from "./catalog.js"

const seriesMetadata = ({
  chapters: _chapters,
  ...series
}: Series & { chapters: readonly unknown[] }) => series
const chapterMetadata = ({ pages: _pages, ...chapter }: Chapter & { pages: readonly unknown[] }) =>
  chapter

export default defineExtension({
  manifest: Schema.decodeUnknownSync(ExtensionManifest)(manifest),
  config: Config,
  validate: (config) => loadCatalog(config).pipe(Effect.asVoid),
  browse: Effect.fn("catalog.browse")(function* (config, query) {
    if (Object.keys(query.filters).length || (query.sort && query.sort !== "title"))
      return yield* sourceError(
        "filter-unsupported",
        "This catalog supports title sorting and no extra filters",
      )
    const catalog = yield* loadCatalog(config)
    return yield* paginate(
      catalog.series
        .map(seriesMetadata)
        .sort((a, b) => a.title.localeCompare(b.title) || a.key.localeCompare(b.key)),
      query,
    )
  }),
  search: Effect.fn("catalog.search")(function* (config, query) {
    if (Object.keys(query.filters).length)
      return yield* sourceError("filter-unsupported", "This catalog does not define extra filters")
    const catalog = yield* loadCatalog(config)
    const text = query.text.toLocaleLowerCase()
    return yield* paginate(
      catalog.series
        .filter((series) =>
          [series.title, ...series.alternateTitles].some((title) =>
            title.toLocaleLowerCase().includes(text),
          ),
        )
        .map(seriesMetadata),
      query,
    )
  }),
  series: Effect.fn("catalog.series")(function* (config, query) {
    const catalog = yield* loadCatalog(config)
    const series = catalog.series.find((series) => series.key === query.seriesKey)
    if (!series)
      return yield* sourceError("series-missing", "Series no longer exists in this catalog")
    return seriesMetadata(series)
  }),
  chapters: Effect.fn("catalog.chapters")(function* (config, query) {
    const catalog = yield* loadCatalog(config)
    const series = catalog.series.find((series) => series.key === query.seriesKey)
    if (!series)
      return yield* sourceError("series-missing", "Series no longer exists in this catalog")
    return yield* paginate(series.chapters.map(chapterMetadata), query)
  }),
  pages: Effect.fn("catalog.pages")(function* (config, query) {
    const catalog = yield* loadCatalog(config)
    const chapter = catalog.series
      .flatMap((series) => series.chapters)
      .find((chapter) => chapter.key === query.chapterKey)
    if (!chapter)
      return yield* sourceError("chapter-missing", "Chapter no longer exists in this catalog")
    return yield* paginate(chapter.pages, query)
  }),
})
