import { Layer } from "effect"
import { TelemetryLive } from "./observability/telemetry"
import { ApiServerLive } from "./server"
import { ApiConfig } from "./services/config"

export const AppLive = ApiServerLive.pipe(
  Layer.provide(TelemetryLive),
  Layer.provide(ApiConfig.layer),
)
