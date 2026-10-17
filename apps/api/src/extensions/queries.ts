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
import { CatalogStore } from "../catalog/store"
import { Missing, RequestError, StorageError } from "../catalog/models"
import { ApiConfig } from "../services/config"
import { ExtensionRegistry } from "./registry"
import { ExtensionHost } from "./host"
import { stateKey } from "./state"
import { resourceStore, type Resource } from "./resources"
type QueryError = Missing | RequestError | StorageError

/** Queries are coalesced and cached for 30 seconds. Raw results stay inside the server. */
export class SourceQueries extends Context.Service<
  SourceQueries,
  {
    readonly query: (
      sourceId: string,
      query: SourceQuery,
    ) => Effect.Effect<SourceResult, QueryError>
    readonly resourceUrl: (
      sourceId: string,
      asset: AssetReference,
      mediaType: string,
    ) => Effect.Effect<string, StorageError>
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
      const resources = yield* resourceStore(config.dataDirectory, key)
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
            grants: { readRoots: source.readRoots, httpOrigins: source.httpOrigins },
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
        resourceUrl: resources.issue,
        resource: resources.resolve,
      })
    }),
  )
}
