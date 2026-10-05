import { Effect } from "effect"
import { HttpApiBuilder } from "effect/http-api"
import { MangaApi } from "../api/api"

export const BasicHandlers = HttpApiBuilder.group(MangaApi, "basic", (handlers) =>
  handlers.handleAll({
    root: () => Effect.succeed({ message: "Chibi Manga API" }),
    health: () => Effect.succeed({ status: "ok" }),
  }),
)
