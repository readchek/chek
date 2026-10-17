import { Effect, Layer } from "effect"
import { HttpRouter, HttpServerResponse } from "effect/http"

function file(path: string | URL, contentType: string) {
  return Effect.promise(async () =>
    HttpServerResponse.fromWeb(
      new Response(Bun.file(path), { headers: { "content-type": contentType } }),
    ),
  )
}

export const InspectorRoutes = Layer.mergeAll(
  HttpRouter.add(
    "GET",
    "/inspect",
    file(new URL("../inspector/index.html", import.meta.url), "text/html; charset=utf-8"),
  ),
  HttpRouter.add(
    "GET",
    "/inspect/app.js",
    file(new URL("../inspector/app.js", import.meta.url), "text/javascript; charset=utf-8"),
  ),
  HttpRouter.add(
    "GET",
    "/inspect/theme.css",
    file(new URL("../inspector/theme.css", import.meta.url), "text/css"),
  ),
  HttpRouter.add(
    "GET",
    "/inspect/oat.css",
    file(new URL(import.meta.resolve("@knadh/oat/oat.min.css")), "text/css"),
  ),
)
