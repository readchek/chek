import { Cache, Duration, Effect, Exit, Layer, Schema, Semaphore } from "effect"
import { HttpRouter, HttpServerRequest, HttpServerResponse } from "effect/http"
import { AssetReference } from "@chek/core"
import { SourceQueries } from "../extensions/queries"
import { CatalogStore } from "../catalog/store"
import { assetPath, storeAsset } from "../catalog/assets"
import { ApiConfig } from "../services/config"
import { Missing, RequestError } from "../catalog/models"

/** Fetch on demand, coalesce concurrent page loads, and store immutable bytes through the import path. */
export const SourceAssetRoutes = Layer.unwrap(
  Effect.gen(function* () {
    const queries = yield* SourceQueries
    const store = yield* CatalogStore
    const config = yield* ApiConfig
    const permits = yield* Semaphore.make(4)
    const cache = yield* Cache.makeWith(
      (key: string) =>
        permits.withPermit(
          Effect.gen(function* () {
            const { sourceId, asset } = Schema.decodeUnknownSync(
              Schema.Struct({ sourceId: Schema.String, asset: AssetReference }),
            )(JSON.parse(key))
            const source = yield* store.call("source", sourceId)
            if (!source) return yield* new Missing({ message: "Source not found" })
            return yield* Effect.tryPromise({
              try: (signal) =>
                storeAsset(
                  {
                    sourceKey: "page",
                    title: "Source page",
                    authors: [],
                    format: "image",
                    mediaType: "application/octet-stream",
                    asset,
                  },
                  source,
                  config.dataDirectory,
                  Math.min(config.maxAssetBytes, 32 * 1024 * 1024),
                  AbortSignal.any([signal, AbortSignal.timeout(config.assetTimeoutMs)]),
                  null,
                ),
              catch: () =>
                new RequestError({
                  code: "source-asset-unavailable",
                  message: "Page could not be loaded. Refresh the source page and retry.",
                }),
            })
          }),
        ),
      {
        capacity: 256,
        timeToLive: (exit) => (Exit.isSuccess(exit) ? Duration.minutes(10) : Duration.zero),
      },
    )

    return HttpRouter.add(
      "GET",
      "/api/v1/source-assets",
      Effect.gen(function* () {
        const request = yield* HttpServerRequest.HttpServerRequest
        const token = new URL(request.url, "http://localhost").searchParams.get("token")
        const resource = yield* queries.resource(token ?? "")
        const asset = yield* Cache.get(
          cache,
          JSON.stringify({ sourceId: resource.sourceId, asset: resource.asset }),
        )
        const file = Bun.file(assetPath(config.dataDirectory, asset.hash))
        const prefix = yield* Effect.promise(() => file.slice(0, 512).bytes())
        const type = imageType(prefix)
        if (!type)
          return yield* new RequestError({
            code: "source-image-invalid",
            message: "Source returned an unsupported image or a non-image response",
          })
        const etag = `"${asset.hash}"`
        const headers = {
          "content-type": type,
          "cache-control": "private, max-age=300",
          "x-content-type-options": "nosniff",
          "content-security-policy": "default-src 'none'; sandbox",
          etag,
        }
        if (request.headers["if-none-match"] === etag)
          return HttpServerResponse.empty({ status: 304, headers })
        return HttpServerResponse.fromWeb(new Response(file, { headers }))
      }).pipe(
        Effect.tapError((error) =>
          Effect.logWarning("Source page delivery failed", {
            code: error._tag === "Missing" ? "source-missing" : error.code,
          }),
        ),
        Effect.catch((error) =>
          Effect.succeed(
            HttpServerResponse.jsonUnsafe(
              {
                code: error._tag === "Missing" ? "source-missing" : error.code,
                message: error.message,
              },
              {
                status:
                  error._tag === "Missing"
                    ? 404
                    : error.code === "source-asset-expired"
                      ? 410
                      : error._tag === "StorageError"
                        ? 503
                        : 502,
              },
            ),
          ),
        ),
        Effect.withSpan("sources.deliverPage"),
      ),
    )
  }),
)

/** Do not serve an upstream login/error page with an extension-supplied image content type. */
function imageType(bytes: Uint8Array): string | undefined {
  const data = Buffer.from(bytes)
  if (data[0] === 0xff && data[1] === 0xd8 && data[2] === 0xff) return "image/jpeg"
  if (data.subarray(0, 8).equals(Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]))) return "image/png"
  if (["GIF87a", "GIF89a"].includes(data.toString("ascii", 0, 6))) return "image/gif"
  if (data.toString("ascii", 0, 4) === "RIFF" && data.toString("ascii", 8, 12) === "WEBP")
    return "image/webp"
  if (data.toString("ascii", 4, 8) === "ftyp" && /avif|avis/.test(data.toString("ascii", 8, 32)))
    return "image/avif"
  if (data.toString("ascii", 0, 2) === "BM") return "image/bmp"
  if (
    data.subarray(0, 4).equals(Buffer.from([73, 73, 42, 0])) ||
    data.subarray(0, 4).equals(Buffer.from([77, 77, 0, 42]))
  )
    return "image/tiff"
  if (/^(?:<\?xml[^>]*>\s*)?<svg[\s>]/.test(data.toString("utf8").trimStart()))
    return "image/svg+xml"
  return undefined
}
