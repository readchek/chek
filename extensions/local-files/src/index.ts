import { defineExtension, ExtensionManifest } from "@chek/extensions"
import { Effect, Schema, Stream } from "effect"
import { LocalConfig, resolveRoot } from "./config"
import { discoverFiles } from "./files"
import manifest from "../extension.json"

export default defineExtension({
  manifest: Schema.decodeUnknownSync(ExtensionManifest)(manifest),
  config: LocalConfig,
  validate: (config) => resolveRoot(config).pipe(Effect.asVoid),
  discover: (config) => Stream.unwrap(resolveRoot(config).pipe(Effect.map(discoverFiles))),
})
