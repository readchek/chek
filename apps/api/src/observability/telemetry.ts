import { Effect, Layer } from "effect"
import { FetchHttpClient } from "effect/http"
import { OtlpLogger, OtlpSerialization, OtlpTracer } from "effect/observability"
import { ApiConfig } from "../services/config"
import { LoggerLive } from "./logger"

export const TelemetryLive = Layer.unwrap(
  Effect.gen(function* () {
    const config = yield* ApiConfig
    const resource = { serviceName: config.serviceName }
    return Layer.merge(
      OtlpLogger.layerFromConfig({ resource }),
      OtlpTracer.layerFromConfig({ resource }),
    ).pipe(
      Layer.provide(
        config.otlpProtocol === "http/json"
          ? OtlpSerialization.layerJson
          : OtlpSerialization.layerProtobuf,
      ),
      Layer.provide(FetchHttpClient.layer),
    )
  }),
).pipe(Layer.provideMerge(LoggerLive))
