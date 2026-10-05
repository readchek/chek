import { Schema } from "effect"
import { HttpApi, HttpApiEndpoint, HttpApiGroup } from "effect/http-api"

export const MangaApi = HttpApi.make("MangaApi").add(
  HttpApiGroup.make("basic").add(
    HttpApiEndpoint.get("root", "/", {
      success: Schema.Struct({ message: Schema.String }),
    }),
    HttpApiEndpoint.get("health", "/health", {
      success: Schema.Struct({ status: Schema.Literal("ok") }),
    }),
  ),
)
