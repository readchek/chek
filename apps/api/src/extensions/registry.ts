import { Context, Effect, Layer, Stream } from "effect"
import { ApiConfig } from "../services/config"
import { RequestError, type SourceInput } from "../catalog/models"
import { ExtensionHost } from "./host"
import { loadInstalledExtensions, type InstalledExtension } from "./installation"
import { normalizeGrants, validateFields } from "./source-validation"

export class ExtensionRegistry extends Context.Service<
  ExtensionRegistry,
  {
    readonly installed: ReadonlyMap<string, InstalledExtension>
    readonly validateSource: (source: SourceInput) => Effect.Effect<SourceInput, RequestError>
  }
>()("@chek/api/ExtensionRegistry") {
  static readonly layer = Layer.effect(
    ExtensionRegistry,
    Effect.gen(function* () {
      const config = yield* ApiConfig
      const host = yield* ExtensionHost
      const installed = yield* loadInstalledExtensions(config)

      const validateSource = Effect.fn("extensions.validateSource")(function* (
        source: SourceInput,
      ) {
        const extension = installed.get(source.extensionId)
        if (!extension)
          return yield* new RequestError({
            code: "extension-missing",
            message: "Extension is not installed",
          })

        yield* validateFields(source, extension.manifest)
        const normalized = yield* normalizeGrants(source)

        yield* Stream.runDrain(
          host.execute(extension, {
            protocolVersion: 1,
            operation: "validate",
            extensionId: extension.manifest.id,
            extensionVersion: extension.manifest.version,
            config: normalized.config,
            grants: { readRoots: normalized.readRoots, httpOrigins: normalized.httpOrigins },
          }),
        ).pipe(
          Effect.mapError(
            (error) =>
              new RequestError({
                code: error.code,
                message: "Extension rejected configuration or grants",
              }),
          ),
        )

        return normalized
      })

      yield* Effect.logInfo("Extensions loaded", {
        extensionIds: [...installed.keys()],
        protocolVersion: 1,
      })
      return ExtensionRegistry.of({ installed, validateSource })
    }),
  )
}
