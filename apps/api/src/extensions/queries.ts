import { AssetReference, SourceQuery, type SourceResult } from "@chek/core"
import {
  Cache,
  Context,
  Duration,
  Effect,
  Exit,
  Layer,
  Schedule,
  Schema,
  Semaphore,
  Stream,
} from "effect"
import { createCipheriv, createDecipheriv, randomBytes } from "node:crypto"
import { CatalogStore } from "../catalog/store"
import { Missing, RequestError, StorageError } from "../catalog/models"
import { ApiConfig } from "../services/config"
import { ExtensionRegistry } from "./registry"
import { ExtensionHost } from "./host"
import { stateKey } from "./state"

const Resource = Schema.Struct({
  sourceId: Schema.String,
  asset: AssetReference,
  mediaType: Schema.String,
  expiresAt: Schema.Number,
})
type Resource = typeof Resource.Type
type QueryError = Missing | RequestError | StorageError

/** Queries are coalesced and cached for 30 seconds. Raw results stay inside the server. */
export class SourceQueries extends Context.Service<
  SourceQueries,
  {
    readonly query: (
      sourceId: string,
      query: SourceQuery,
    ) => Effect.Effect<SourceResult, QueryError>
    readonly resourceUrl: (sourceId: string, asset: AssetReference, mediaType: string) => string
    readonly resource: (token: string) => Effect.Effect<Resource, RequestError>
  }
>()("@chek/api/SourceQueries") {
  static readonly layer = Layer.effect(
    SourceQueries,
    Effect.gen(function* () {
      const store = yield* CatalogStore
      const registry = yield* ExtensionRegistry
      const host = yield* ExtensionHost
      const config = yield* ApiConfig
      const key = yield* Effect.promise(() => stateKey(config.dataDirectory))
      const permits = yield* Semaphore.make(4)

      const lookup = Effect.fn("sources.query")(function* (cacheKey: string) {
        const { sourceId, query } = Schema.decodeUnknownSync(
          Schema.Struct({ sourceId: Schema.String, query: SourceQuery }),
        )(JSON.parse(cacheKey))
        const source = yield* store.call("source", sourceId)
        if (!source) return yield* new Missing({ message: "Source not found" })
        const extension = registry.installed.get(source.extensionId)
        if (!extension) return yield* new Missing({ message: "Source extension is not installed" })
        if (!extension.manifest.capabilities.includes(query.operation))
          return yield* new RequestError({
            code: "capability-missing",
            message: `Source does not support ${query.operation}`,
          })
        yield* Effect.annotateCurrentSpan({
          "chek.source.id": sourceId,
          "chek.extension.id": source.extensionId,
          "chek.source.operation": query.operation,
        })
        let result: SourceResult | undefined
        yield* host
          .execute(extension, {
            protocolVersion: 1,
            operation: "query",
            sourceId,
            extensionId: source.extensionId,
            extensionVersion: extension.manifest.version,
            config: source.config,
            grants: source,
            query,
          })
          .pipe(
            Stream.runForEach((event) =>
              Effect.sync(() => {
                if (event.type === "result") result = event.result
              }),
            ),
            Effect.retry({
              times: 2,
              schedule: Schedule.exponential("500 millis"),
              while: (error) => error.retryable,
            }),
            Effect.tapError((error) =>
              Effect.logWarning("Source query failed", {
                sourceId,
                operation: query.operation,
                code: error.code,
              }),
            ),
            Effect.mapError((error) =>
              error.retryable
                ? new StorageError({
                    code: error.code,
                    message: "Source is temporarily unavailable",
                  })
                : new RequestError({ code: error.code, message: error.message }),
            ),
          )
        if (!result)
          return yield* new RequestError({
            code: "source-result-missing",
            message: "Source returned no result",
          })
        return result
      })
      const cache = yield* Cache.makeWith((key: string) => permits.withPermit(lookup(key)), {
        capacity: 128,
        timeToLive: (exit) => (Exit.isSuccess(exit) ? Duration.seconds(30) : Duration.zero),
      })

      return SourceQueries.of({
        query: (sourceId, query) => Cache.get(cache, JSON.stringify({ sourceId, query })),
        resourceUrl: (sourceId, asset, mediaType) => {
          const iv = randomBytes(12)
          const cipher = createCipheriv("aes-256-gcm", key, iv)
          cipher.setAAD(Buffer.from("chek-source-resource-v1"))
          const body = Buffer.concat([
            cipher.update(
              JSON.stringify({ sourceId, asset, mediaType, expiresAt: Date.now() + 600_000 }),
            ),
            cipher.final(),
          ])
          const token = Buffer.concat([iv, cipher.getAuthTag(), body]).toString("base64url")
          return `/api/v1/source-assets/${token}`
        },
        resource: (token) =>
          Effect.try({
            try: () => {
              if (token.length > 32768) throw new Error("Oversized token")
              const bytes = Buffer.from(token, "base64url")
              const cipher = createDecipheriv("aes-256-gcm", key, bytes.subarray(0, 12))
              cipher.setAAD(Buffer.from("chek-source-resource-v1"))
              cipher.setAuthTag(bytes.subarray(12, 28))
              const resource = Schema.decodeUnknownSync(Resource)(
                JSON.parse(
                  Buffer.concat([cipher.update(bytes.subarray(28)), cipher.final()]).toString(),
                ),
              )
              if (resource.expiresAt <= Date.now()) throw new Error("Expired token")
              return resource
            },
            catch: () =>
              new RequestError({
                code: "source-asset-expired",
                message: "Reload the source page to obtain a fresh asset URL",
              }),
          }),
      })
    }),
  )
}
