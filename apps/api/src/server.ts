import { BunHttpServer } from "@effect/platform-bun"
import { Effect, Layer } from "effect"
import { HttpRouter } from "effect/http"
import { HttpApiBuilder } from "effect/http-api"
import { MangaApi } from "./api/api"
import { BasicHandlers } from "./handlers/basic"
import { ApiConfig } from "./services/config"

const Routes = HttpApiBuilder.layer(MangaApi).pipe(Layer.provide(BasicHandlers))

export const ApiServerLive = Layer.unwrap(
  Effect.gen(function* () {
    const config = yield* ApiConfig
    return HttpRouter.serve(Routes).pipe(
      Layer.provide(BunHttpServer.layer({ hostname: config.host, port: config.port })),
    )
  }),
)
