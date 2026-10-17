import { ExtensionCollection } from "@chek/extensions"
import { Context, Effect, Layer, Schema } from "effect"
import { readdir } from "node:fs/promises"
import { resolve } from "node:path"
import { ApiConfig } from "../services/config"
import { RequestError } from "../catalog/models"

/** Operator-supplied catalogs can describe public or private npm releases. */
export class ExtensionCollections extends Context.Service<
  ExtensionCollections,
  {
    readonly collections: readonly ExtensionCollection[]
  }
>()("@chek/api/ExtensionCollections") {
  static readonly layer = Layer.effect(
    ExtensionCollections,
    Effect.gen(function* () {
      const config = yield* ApiConfig
      if (!config.collectionsDirectory) return ExtensionCollections.of({ collections: [] })

      const collections = yield* Effect.tryPromise({
        try: async () => {
          const files = (await readdir(config.collectionsDirectory, { withFileTypes: true }))
            .filter((file) => file.isFile() && file.name.endsWith(".json"))
            .sort((left, right) => left.name.localeCompare(right.name))
          if (files.length > 100) throw new Error("Too many collections")

          const result: ExtensionCollection[] = []
          for (const file of files) {
            const input = Bun.file(resolve(config.collectionsDirectory, file.name))
            if (input.size > 1024 * 1024) throw new Error("Collection exceeds 1 MiB")
            const collection = Schema.decodeUnknownSync(ExtensionCollection)(await input.json(), {
              onExcessProperty: "error",
            })
            if (result.some((existing) => existing.id === collection.id))
              throw new Error("Duplicate collection")
            const releases = collection.extensions.map(
              (release) => `${release.id}@${release.version}`,
            )
            if (new Set(releases).size !== releases.length) throw new Error("Duplicate release")
            result.push(collection)
          }
          return result
        },
        catch: () =>
          new RequestError({
            code: "collection-invalid",
            message: "Cannot load extension collections; check catalog schemas and duplicate IDs",
          }),
      })

      yield* Effect.logInfo("Extension collections loaded", { count: collections.length })
      return ExtensionCollections.of({ collections })
    }),
  )
}
