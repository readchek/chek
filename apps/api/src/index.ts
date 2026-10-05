import { BunHttpServer, BunRuntime } from "@effect/platform-bun"
import { Effect, Layer, Schema } from "effect"
import { HttpRouter } from "effect/http"
import { HttpApi, HttpApiBuilder, HttpApiEndpoint, HttpApiGroup } from "effect/http-api"
import { ApiConfig } from "./config"
import { Observability } from "./observability"

const Api = HttpApi.make("MangaApi").add(
  HttpApiGroup.make("basic").add(
    HttpApiEndpoint.get("root", "/", {
      success: Schema.Struct({ message: Schema.String }),
    }),
    HttpApiEndpoint.get("health", "/health", {
      success: Schema.Struct({ status: Schema.Literal("ok") }),
    }),
  ),
)

const Handlers = HttpApiBuilder.group(Api, "basic", (handlers) =>
  handlers.handleAll({
    root: () => Effect.succeed({ message: "Chibi Manga API" }),
    health: () => Effect.succeed({ status: "ok" }),
  }),
)

const Routes = HttpApiBuilder.layer(Api).pipe(Layer.provide(Handlers))

const Server = Layer.unwrap(
  Effect.gen(function* () {
    const config = yield* ApiConfig
    return HttpRouter.serve(Routes).pipe(
      Layer.provide(BunHttpServer.layer({ hostname: config.host, port: config.port })),
    )
  }),
)

const Main = Server.pipe(Layer.provide(Observability), Layer.provide(ApiConfig.layer))

Layer.launch(Main).pipe(BunRuntime.runMain)
