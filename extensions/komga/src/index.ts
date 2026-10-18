import { defineExtension, ExtensionManifest } from "@chek/extensions"
import { Effect, Schema } from "effect"
import manifest from "../extension.json" with { type: "json" }
import { Config, validateConfig } from "./client.js"
import { chapters, discover, getSeries, listSeries, pages, validateAccess } from "./operations.js"
import { seriesModel } from "./mapping.js"

export default defineExtension({
  manifest: Schema.decodeUnknownSync(ExtensionManifest)(manifest),
  config: Config,
  validate: (config) => validateConfig(config).pipe(Effect.andThen(validateAccess(config))),
  browse: listSeries,
  search: listSeries,
  series: (config, query) =>
    getSeries(config, query.seriesKey).pipe(Effect.map((series) => seriesModel(config, series))),
  chapters,
  pages,
  discover,
})
