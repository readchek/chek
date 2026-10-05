import { Effect, Layer, Logger, References } from "effect"
import { ApiConfig } from "../services/config"

export const LoggerLive = Layer.unwrap(
  Effect.gen(function* () {
    const config = yield* ApiConfig
    return Layer.merge(
      Logger.layer([config.logFormat === "json" ? Logger.consoleJson : Logger.consolePrettyTty()]),
      Layer.succeed(References.MinimumLogLevel, config.logLevel),
    )
  }),
)
