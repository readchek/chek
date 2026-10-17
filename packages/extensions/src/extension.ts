import { AssetGrants, Text } from "@chek/core"
import { Context, Effect, Schema, Stream } from "effect"
import { type ExtensionConfig, ExtensionManifest } from "./manifest.js"
import { DiscoveryEvent } from "./protocol.js"

export class ExtensionError extends Schema.TaggedError<ExtensionError>()("ExtensionError", {
  code: Text,
  message: Text,
  retryable: Schema.Boolean,
}) {}

/** Supplied by the host for each execution; interruption is carried by Effect. */
export class ExtensionContext extends Context.Service<
  ExtensionContext,
  {
    readonly grants: AssetGrants
  }
>()("@chek/extensions/ExtensionContext") {}

export interface Extension {
  readonly manifest: ExtensionManifest
  readonly validate: (
    config: ExtensionConfig,
  ) => Effect.Effect<void, ExtensionError, ExtensionContext>
  readonly discover: (
    config: ExtensionConfig,
  ) => Stream.Stream<DiscoveryEvent, ExtensionError, ExtensionContext>
}

export interface ExtensionDefinition<Config> {
  readonly manifest: ExtensionManifest
  readonly config: Schema.Codec<Config, unknown>
  readonly validate: (config: Config) => Effect.Effect<void, ExtensionError, ExtensionContext>
  readonly discover: (
    config: Config,
  ) => Stream.Stream<DiscoveryEvent, ExtensionError, ExtensionContext>
}

/** Decode at the boundary, then keep configuration typed throughout the extension. */
export function defineExtension<Config>(definition: ExtensionDefinition<Config>): Extension {
  const manifest = Schema.decodeUnknownSync(ExtensionManifest)(definition.manifest, {
    onExcessProperty: "error",
  })
  const decodeConfig = Schema.decodeUnknownEffect(definition.config)
  const decodeEvent = Schema.decodeUnknownEffect(DiscoveryEvent)

  const configuration = (input: ExtensionConfig) =>
    decodeConfig(input, { onExcessProperty: "error" }).pipe(
      Effect.mapError(
        () =>
          new ExtensionError({
            code: "configuration-invalid",
            message: "Extension configuration is invalid",
            retryable: false,
          }),
      ),
    )

  return {
    manifest,
    validate: Effect.fn("extension.validate")(function* (input) {
      yield* definition.validate(yield* configuration(input))
    }),
    discover: (input) =>
      Stream.unwrap(
        Effect.gen(function* () {
          const config = yield* configuration(input)
          yield* definition.validate(config)

          return definition.discover(config).pipe(
            Stream.mapEffect((event) =>
              decodeEvent(event, { onExcessProperty: "error" }).pipe(
                Effect.mapError(
                  () =>
                    new ExtensionError({
                      code: "publication-invalid",
                      message: "Extension produced an invalid discovery event",
                      retryable: false,
                    }),
                ),
              ),
            ),
          )
        }),
      ),
  }
}
