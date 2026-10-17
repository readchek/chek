import { AssetGrants, SourceQuery, SourceResult, type QueryFor, type ResultFor } from "@chek/core"
import { Context, Effect, Schema, Stream } from "effect"
import { type ExtensionConfig, ExtensionManifest } from "./manifest.js"
import { DiscoveryEvent } from "./protocol.js"
import { ExtensionError, unsupported } from "./errors.js"
import { SourceHttp, SourceState } from "./services.js"

/** Immutable permissions for this invocation. Effect carries cancellation through all services. */
export class ExtensionContext extends Context.Service<
  ExtensionContext,
  {
    readonly grants: AssetGrants
    /** Last host-acknowledged import cursor; null means start a full scan. */
    readonly checkpoint: string | null
  }
>()("@chek/extensions/ExtensionContext") {}

export type ExtensionServices = ExtensionContext | SourceHttp | SourceState

/** Optional operations must match the capabilities declared in extension.json. */
export type SourceHandlers<Config> = {
  readonly [K in SourceQuery["operation"]]?: (
    config: Config,
    query: QueryFor<K>,
  ) => Effect.Effect<ResultFor<K>, ExtensionError, ExtensionServices>
}

/** Implement source operations here. The SDK owns input/output validation. */
export interface ExtensionDefinition<Config> extends SourceHandlers<Config> {
  readonly manifest: ExtensionManifest
  readonly config: Schema.Codec<Config, unknown>
  /** Validate configuration without importing content. Fail with a safe, actionable error code. */
  readonly validate: (config: Config) => Effect.Effect<void, ExtensionError, ExtensionServices>
  /** Stream original-file candidates for durable import. Remote browsing does not require this operation. */
  readonly discover?: (
    config: Config,
  ) => Stream.Stream<DiscoveryEvent, ExtensionError, ExtensionServices>
}

/** Runtime contract consumed by the host. Authors normally export defineExtension(...). */
export interface Extension {
  readonly manifest: ExtensionManifest
  readonly validate: (
    config: ExtensionConfig,
  ) => Effect.Effect<void, ExtensionError, ExtensionServices>
  readonly discover: (
    config: ExtensionConfig,
  ) => Stream.Stream<DiscoveryEvent, ExtensionError, ExtensionServices>
  readonly query: (
    config: ExtensionConfig,
    query: SourceQuery,
  ) => Effect.Effect<SourceResult, ExtensionError, ExtensionServices>
}

/**
 * Build a validated extension. Config is decoded before every operation; results are
 * checked before crossing the process boundary. Provide your own service layers
 * inside handlers, leaving only ExtensionServices for the host to supply.
 */
export function defineExtension<Config>(definition: ExtensionDefinition<Config>): Extension {
  const manifest = Schema.decodeUnknownSync(ExtensionManifest)(definition.manifest, {
    onExcessProperty: "error",
  })
  const operations = ["discover", "browse", "search", "series", "chapters", "pages"] as const

  for (const operation of operations) {
    if (manifest.capabilities.includes(operation) !== (definition[operation] !== undefined)) {
      throw new Error(`Manifest capability and handler disagree: ${operation}`)
    }
  }
  if (new Set(manifest.capabilities).size !== manifest.capabilities.length)
    throw new Error("Duplicate capability")

  const configuration = (input: ExtensionConfig) =>
    Schema.decodeUnknownEffect(definition.config)(input, { onExcessProperty: "error" }).pipe(
      Effect.mapError(
        () =>
          new ExtensionError({
            code: "configuration-invalid",
            message: "Extension configuration is invalid",
            retryable: false,
          }),
      ),
    )

  const execute = (
    config: Config,
    query: SourceQuery,
  ): Effect.Effect<SourceResult, ExtensionError, ExtensionServices> => {
    switch (query.operation) {
      case "browse":
        return definition.browse
          ? definition
              .browse(config, query)
              .pipe(Effect.map((data) => ({ operation: "browse" as const, data })))
          : Effect.fail(unsupported("browse"))
      case "search":
        return definition.search
          ? definition
              .search(config, query)
              .pipe(Effect.map((data) => ({ operation: "search" as const, data })))
          : Effect.fail(unsupported("search"))
      case "series":
        return definition.series
          ? definition
              .series(config, query)
              .pipe(Effect.map((data) => ({ operation: "series" as const, data })))
          : Effect.fail(unsupported("series"))
      case "chapters":
        return definition.chapters
          ? definition
              .chapters(config, query)
              .pipe(Effect.map((data) => ({ operation: "chapters" as const, data })))
          : Effect.fail(unsupported("chapters"))
      case "pages":
        return definition.pages
          ? definition
              .pages(config, query)
              .pipe(Effect.map((data) => ({ operation: "pages" as const, data })))
          : Effect.fail(unsupported("pages"))
    }
  }

  return {
    manifest,
    validate: Effect.fn("extension.validate")(function* (input) {
      yield* definition.validate(yield* configuration(input))
    }),
    discover: (input) =>
      Stream.unwrap(
        Effect.gen(function* () {
          if (!definition.discover) return yield* unsupported("discover")
          const config = yield* configuration(input)
          yield* definition.validate(config)
          return definition.discover(config).pipe(
            Stream.mapEffect((event) =>
              Schema.decodeUnknownEffect(DiscoveryEvent)(event, {
                onExcessProperty: "error",
              }).pipe(
                Effect.mapError(
                  () =>
                    new ExtensionError({
                      code: "publication-invalid",
                      message: "Invalid discovery record",
                      retryable: false,
                    }),
                ),
              ),
            ),
          )
        }),
      ),
    query: Effect.fn("extension.query")(function* (input, query) {
      const config = yield* configuration(input)
      const validQuery = yield* Schema.decodeUnknownEffect(SourceQuery)(query, {
        onExcessProperty: "error",
      }).pipe(
        Effect.mapError(
          () =>
            new ExtensionError({
              code: "query-invalid",
              message: "Invalid source query",
              retryable: false,
            }),
        ),
      )
      const result = yield* execute(config, validQuery)
      const valid = yield* Schema.decodeUnknownEffect(SourceResult)(result, {
        onExcessProperty: "error",
      }).pipe(
        Effect.mapError(
          () =>
            new ExtensionError({
              code: "source-result-invalid",
              message: "Invalid source result",
              retryable: false,
            }),
        ),
      )

      if (
        valid.operation === "series" &&
        validQuery.operation === "series" &&
        valid.data.key !== validQuery.seriesKey
      ) {
        return yield* new ExtensionError({
          code: "series-identity-invalid",
          message: "Source returned a different series",
          retryable: false,
        })
      }
      if (valid.operation !== "series" && validQuery.operation !== "series") {
        if (
          valid.data.items.length > validQuery.limit ||
          (valid.data.nextCursor !== null && valid.data.nextCursor === validQuery.cursor)
        ) {
          return yield* new ExtensionError({
            code: "pagination-invalid",
            message: "Source exceeded the page limit or repeated its cursor",
            retryable: false,
          })
        }
      }
      if (
        valid.operation === "chapters" &&
        validQuery.operation === "chapters" &&
        valid.data.items.some((chapter) => chapter.seriesKey !== validQuery.seriesKey)
      ) {
        return yield* new ExtensionError({
          code: "chapter-identity-invalid",
          message: "Source returned a chapter from another series",
          retryable: false,
        })
      }
      if (
        valid.operation === "browse" ||
        valid.operation === "search" ||
        valid.operation === "chapters"
      ) {
        const keys = valid.data.items.map((item) => item.key)
        if (new Set(keys).size !== keys.length)
          return yield* new ExtensionError({
            code: "source-identity-invalid",
            message: "Source returned duplicate identities",
            retryable: false,
          })
      }

      if (
        valid.operation === "pages" &&
        valid.data.items.some(
          (page, index, items) => index > 0 && page.index <= items[index - 1]!.index,
        )
      ) {
        return yield* new ExtensionError({
          code: "page-order-invalid",
          message: "Page indices must be unique and increasing",
          retryable: false,
        })
      }
      return valid
    }),
  }
}
