import { BunHttpServer, BunRuntime } from "@effect/platform-bun"
import { Effect, Layer, Schema, Logger } from "effect"
import { HttpRouter } from "effect/http"
import { HttpApi, HttpApiBuilder, HttpApiEndpoint, HttpApiGroup } from "effect/http-api"

const Api = HttpApi.make("MangaApi").add(
  HttpApiGroup.make("basic").add(
    HttpApiEndpoint.get("health", "/health", {
      success: Schema.Struct({ status: Schema.Literal("ok") }),
    }),
    HttpApiEndpoint.get("hello", "/hello/:name", {
      params: { name: Schema.String },
      success: Schema.Struct({ message: Schema.String }),
    }),
  ),
)

const Handlers = HttpApiBuilder.group(Api, "basic", (handlers) =>
  handlers.handleAll({
    health: () => Effect.succeed({ status: "ok" as const }),
    hello: ({ params }) => Effect.succeed({ message: `Hello, ${params.name}!` }),
  }),
)

const Routes = HttpApiBuilder.layer(Api).pipe(Layer.provide(Handlers))

const Server = HttpRouter.serve(Routes).pipe(
  Layer.provide(
    BunHttpServer.layer({
      hostname: "127.0.0.1",
      port: process.env.PORT ? parseInt(process.env.PORT) : 3000,
    }),
  ),
)

const LoggerLayer = Logger.layer([Logger.consolePrettyTty()])

Layer.launch(Server).pipe(Effect.provide(LoggerLayer), BunRuntime.runMain)
