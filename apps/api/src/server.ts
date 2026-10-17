import { BunHttpServer } from "@effect/platform-bun"
import { Effect, Layer } from "effect"
import { HttpRouter } from "effect/http"
import { HttpApiBuilder } from "effect/http-api"
import { MangaApi } from "./api/api"
import { BasicHandlers } from "./handlers/basic"
import { ApiConfig } from "./services/config"
import { CatalogHandlers } from "./handlers/catalog"
import { DeliveryRoutes } from "./handlers/delivery"
import { Guard } from "./handlers/guard"
import { InspectorRoutes } from "./handlers/inspector"
import { SourceAssetRoutes } from "./handlers/source-assets"

const Routes = Layer.mergeAll(
  HttpApiBuilder.layer(MangaApi, { openapiPath: "/api/openapi.json" }).pipe(
    Layer.provide([BasicHandlers, CatalogHandlers]),
  ),
  DeliveryRoutes,
  SourceAssetRoutes,
  InspectorRoutes,
).pipe(Layer.provide(Guard))

export const ApiServerLive = Layer.unwrap(
  Effect.gen(function* () {
    const config = yield* ApiConfig
    return HttpRouter.serve(Routes).pipe(
      Layer.provide(BunHttpServer.layer({ hostname: config.host, port: config.port })),
    )
  }),
)
