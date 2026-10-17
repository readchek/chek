import { Effect } from "effect"
import { HttpRouter, HttpServerRequest, HttpServerResponse } from "effect/http"
import { CatalogStore } from "../catalog/store"
import { assetPath } from "../catalog/assets"
import { ApiConfig } from "../services/config"

export const DeliveryRoutes = HttpRouter.add(
  "GET",
  "/api/v1/assets/:hash",
  Effect.gen(function* () {
    const { hash } = yield* HttpRouter.params
    if (!hash || !/^[a-f0-9]{64}$/.test(hash)) return HttpServerResponse.empty({ status: 404 })
    const store = yield* CatalogStore
    const config = yield* ApiConfig
    const request = yield* HttpServerRequest.HttpServerRequest
    const asset = yield* store.call("asset", hash)
    if (!asset) return HttpServerResponse.empty({ status: 404 })
    const file = Bun.file(assetPath(config.dataDirectory, hash))
    if (!(yield* Effect.promise(() => file.exists())))
      return HttpServerResponse.jsonUnsafe(
        { code: "asset-unavailable", message: "Stored asset is missing" },
        { status: 503 },
      )
    if (file.size !== asset.byteLength)
      return HttpServerResponse.jsonUnsafe(
        { code: "asset-integrity", message: "Stored asset size does not match the catalog" },
        { status: 503 },
      )
    const etag = `"${hash}"`
    const headers: Record<string, string> = {
      "accept-ranges": "bytes",
      etag,
      "cache-control": "private, max-age=31536000, immutable",
      "content-type": "application/octet-stream",
      "x-content-type-options": "nosniff",
      "content-disposition": "attachment",
      "content-length": String(asset.byteLength),
    }
    if (
      request.headers["if-none-match"]
        ?.split(",")
        .some((value) => [etag, `W/${etag}`, "*"].includes(value.trim()))
    ) {
      delete headers["content-length"]
      return HttpServerResponse.empty({ status: 304, headers })
    }
    const range = request.headers.range
    if (range && (!request.headers["if-range"] || request.headers["if-range"] === etag)) {
      const match = /^bytes=(\d*)-(\d*)$/.exec(range)
      if (!match || (!match[1] && !match[2]))
        return HttpServerResponse.empty({
          status: 416,
          headers: { "content-range": `bytes */${asset.byteLength}`, etag },
        })
      const suffix = !match[1]
      const first = Number(match[1] || match[2])
      const last = match[2] && !suffix ? Number(match[2]) : asset.byteLength - 1
      const start = suffix ? Math.max(0, asset.byteLength - first) : first
      const end = Math.min(last, asset.byteLength - 1)
      if (
        !Number.isSafeInteger(first) ||
        !Number.isSafeInteger(last) ||
        start > end ||
        (suffix && first === 0)
      ) {
        return HttpServerResponse.empty({
          status: 416,
          headers: { "content-range": `bytes */${asset.byteLength}`, etag },
        })
      }
      headers["content-range"] = `bytes ${start}-${end}/${asset.byteLength}`
      headers["content-length"] = String(end - start + 1)
      return HttpServerResponse.fromWeb(
        new Response(file.slice(start, end + 1), { status: 206, headers }),
      )
    }
    return HttpServerResponse.fromWeb(new Response(file, { headers }))
  }).pipe(
    Effect.catch(() =>
      Effect.succeed(
        HttpServerResponse.jsonUnsafe({ code: "storage-unavailable" }, { status: 503 }),
      ),
    ),
    Effect.withSpan("assets.deliver"),
  ),
)
