import { Effect, Layer, Logger, References } from "effect"
import { FetchHttpClient } from "effect/http"
import { OtlpLogger, OtlpSerialization, OtlpTracer } from "effect/observability"
import { ApiConfig } from "./config"

export const Observability = Layer.unwrap(
  Effect.gen(function* () {
    const config = yield* ApiConfig
    const resource = { serviceName: config.serviceName }
    const consoleLogger = Logger.layer([
      config.logFormat === "json" ? Logger.consoleJson : Logger.consolePrettyTty(),
    ])

    const exporters = Layer.merge(
      OtlpLogger.layerFromConfig({ resource }),
      OtlpTracer.layerFromConfig({ resource }),
    ).pipe(
      Layer.provide(
        config.otlpProtocol === "http/json"
          ? OtlpSerialization.layerJson
          : OtlpSerialization.layerProtobuf,
      ),
      Layer.provide(FetchHttpClient.layer),
      Layer.provide(consoleLogger),
    )

    return Layer.merge(exporters, Layer.succeed(References.MinimumLogLevel, config.logLevel))
  }),
)
