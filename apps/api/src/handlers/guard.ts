import { timingSafeEqual } from "node:crypto"
import { Effect, Metric } from "effect"
import { HttpRouter, HttpServerRequest, HttpServerResponse } from "effect/http"
import { ApiConfig } from "../services/config"

const requestDuration = Metric.histogram("chek.http.handler_duration_ms", {
  boundaries: [5, 10, 25, 50, 100, 250, 1000, 5000],
})
const requestFailures = Metric.counter("chek.http.failures")

export const Guard = HttpRouter.middleware(
  Effect.gen(function* () {
    const config = yield* ApiConfig
    const loopback = ["127.0.0.1", "localhost", "::1"].includes(config.host)
    if (!loopback && config.apiToken.length < 32)
      return yield* Effect.die(
        new Error(
          "CHEK_API_TOKEN must contain at least 32 characters when binding outside loopback",
        ),
      )
    return (next) =>
      Effect.gen(function* () {
        const request = yield* HttpServerRequest.HttpServerRequest
        const started = performance.now()
        const span = yield* Effect.currentSpan
        const fail = (status: number, code: string) =>
          HttpServerResponse.jsonUnsafe(
            { code, traceId: span.traceId },
            { status, headers: { "cache-control": "no-store", "x-trace-id": span.traceId } },
          )
        const host = request.headers.host
        if (
          loopback &&
          host &&
          ![config.host, "localhost", "127.0.0.1", "[::1]"].some(
            (name) => host === `${name}:${config.port}`,
          )
        )
          return fail(403, "host-forbidden")
        const origin = request.headers.origin
        if (origin && origin !== `http://${host}` && origin !== `https://${host}`)
          return fail(403, "origin-forbidden")
        if (request.headers["sec-fetch-site"] === "cross-site")
          return fail(403, "cross-site-forbidden")
        if (request.url.startsWith("/api/") && config.apiToken) {
          const actual = Buffer.from(request.headers.authorization ?? "")
          const expected = Buffer.from(`Bearer ${config.apiToken}`)
          if (actual.length !== expected.length || !timingSafeEqual(actual, expected))
            return fail(401, "unauthorized")
        }
        const response = yield* next
        yield* Metric.update(requestDuration, performance.now() - started)
        if (response.status >= 500) {
          yield* Metric.update(requestFailures, 1)
          yield* Effect.logError("HTTP request failed", {
            status: response.status,
            method: request.method,
            traceId: span.traceId,
          })
        }
        return response.pipe(
          HttpServerResponse.setHeaders({
            "x-trace-id": span.traceId,
            "x-content-type-options": "nosniff",
            "referrer-policy": "no-referrer",
            "content-security-policy":
              "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; object-src 'none'; base-uri 'none'; frame-ancestors 'none'; form-action 'self'",
            ...(request.url.startsWith("/api/") && !request.url.startsWith("/api/v1/assets/")
              ? { "cache-control": "no-store" }
              : {}),
          }),
        )
      })
  }),
).layer
