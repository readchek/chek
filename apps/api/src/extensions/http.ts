import { ExtensionError, SourceHttp, type SourceHttpRequest } from "@chek/extensions"
import { Duration, Effect, Layer, Schema, Stream } from "effect"
import { FetchHttpClient, HttpClient, HttpClientRequest } from "effect/http"
import type { AssetGrants } from "@chek/core"
import type { SessionStore } from "./state"

const Cookie = Schema.Struct({
  name: Schema.String,
  value: Schema.String,
  path: Schema.String,
  secure: Schema.Boolean,
  expires: Schema.Number,
})
const Jar = Schema.Array(Cookie)

const failure = (code: string, retryable = false) =>
  new ExtensionError({
    code,
    message: "Source HTTP request could not be completed",
    retryable,
  })

/** Host cookies are deliberately origin-scoped, even when a server sets Domain=.example.com. */
export function cookieHeader(store: SessionStore, url: URL): string {
  const cookies = Schema.decodeUnknownSync(Jar)(store.get(`cookies:${url.origin}`) ?? [])
  return cookies
    .filter(
      (cookie) =>
        cookie.expires > Date.now() &&
        (!cookie.secure || url.protocol === "https:") &&
        (url.pathname === cookie.path ||
          url.pathname.startsWith(cookie.path.endsWith("/") ? cookie.path : `${cookie.path}/`)),
    )
    .sort((a, b) => b.path.length - a.path.length)
    .map((cookie) => `${cookie.name}=${cookie.value}`)
    .join("; ")
}

/** All requests share an origin pacing ledger; redirects are checked before opening a connection. */
export const sourceHttpLayer = (store: SessionStore, grants: AssetGrants) =>
  Layer.effect(
    SourceHttp,
    Effect.gen(function* () {
      const client = yield* HttpClient.HttpClient

      const request = Effect.fn("source.http")(function* (input: SourceHttpRequest) {
        let target = yield* Effect.try({
          try: () => new URL(input.url),
          catch: () => failure("http-url"),
        })
        const initialOrigin = target.origin
        let method = input.method ?? "GET"
        let body = input.body
        if (body && Buffer.byteLength(body) > 1024 * 1024) return yield* failure("http-body-limit")
        for (let redirects = 0; redirects <= 5; redirects++) {
          if (
            !grants.httpOrigins.includes(target.origin) ||
            target.username ||
            target.password ||
            !["http:", "https:"].includes(target.protocol)
          ) {
            return yield* failure("http-grant")
          }
          yield* Effect.annotateCurrentSpan({
            "server.address": target.hostname,
            "http.request.method": method,
          })
          const wait = yield* Effect.try({
            try: () => store.reserve(target.origin),
            catch: () => failure("source-state"),
          })
          yield* Effect.sleep(wait)
          const cookies = yield* Effect.try({
            try: () => cookieHeader(store, target),
            catch: () => failure("source-state"),
          })
          const headers = new Headers(target.origin === initialOrigin ? input.headers : undefined)
          if (cookies && !headers.has("cookie")) headers.set("cookie", cookies)
          headers.delete("host")
          headers.delete("content-length")
          let outgoing = HttpClientRequest.make(method)(target.href, {
            headers: Object.fromEntries(headers),
          })
          if (body !== undefined)
            outgoing = HttpClientRequest.bodyText(
              outgoing,
              body,
              headers.get("content-type") ?? "application/json",
            )

          const response = yield* client
            .execute(outgoing)
            .pipe(Effect.mapError(() => failure("http-network", method === "GET")))
          yield* Effect.annotateCurrentSpan({ "http.response.status_code": response.status })
          yield* Effect.try({
            try: () =>
              store.update(`cookies:${target.origin}`, (previous) => {
                let jar = [...Schema.decodeUnknownSync(Jar)(previous ?? [])].filter(
                  (cookie) => cookie.expires > Date.now(),
                )
                for (const cookie of Object.values(response.cookies.cookies)) {
                  const options = cookie.options
                  const path = options?.path?.startsWith("/")
                    ? options.path
                    : target.pathname.slice(0, target.pathname.lastIndexOf("/") + 1) || "/"
                  const expires =
                    options?.maxAge !== undefined
                      ? Date.now() + Duration.toMillis(Duration.fromInputUnsafe(options.maxAge))
                      : (options?.expires?.getTime() ?? Date.now() + 86400_000)
                  jar = jar.filter((entry) => entry.name !== cookie.name || entry.path !== path)
                  if (expires > Date.now())
                    jar.push({
                      name: cookie.name,
                      value: cookie.valueEncoded,
                      path,
                      secure: options?.secure ?? false,
                      expires,
                    })
                }
                return jar.slice(-128)
              }),
            catch: () => failure("source-state"),
          })

          if ([301, 302, 303, 307, 308].includes(response.status)) {
            const location = response.headers.location
            if (!location) return yield* failure("http-redirect")
            const next = yield* Effect.try({
              try: () => new URL(location, target),
              catch: () => failure("http-redirect"),
            })
            if (next.origin !== target.origin && body !== undefined)
              return yield* failure("http-redirect-body")
            if (
              response.status === 303 ||
              ([301, 302].includes(response.status) && method === "POST")
            ) {
              method = "GET"
              body = undefined
            }
            target = next
            continue
          }

          if (response.status === 429 || response.status >= 500) {
            const raw = response.headers["retry-after"]
            const seconds = raw ? Number(raw) : NaN
            const delay = raw
              ? Number.isFinite(seconds)
                ? seconds * 1000
                : Date.parse(raw) - Date.now()
              : 0
            if (delay > 0)
              yield* Effect.try({
                try: () => store.reserve(target.origin, Math.min(delay, 60_000)),
                catch: () => failure("source-state"),
              })
            return yield* failure(`http-${response.status}`, method === "GET")
          }

          const chunks: Uint8Array[] = []
          let bytes = 0
          yield* Stream.runForEach(response.stream, (chunk) => {
            bytes += chunk.byteLength
            if (bytes > 4 * 1024 * 1024) return Effect.fail(failure("http-response-limit"))
            chunks.push(chunk)
            return Effect.void
          }).pipe(
            Effect.mapError((error) =>
              error instanceof ExtensionError ? error : failure("http-network", method === "GET"),
            ),
          )
          return {
            status: response.status,
            url: target.href,
            headers: Object.fromEntries(
              Object.entries(response.headers).filter(([name]) => name !== "set-cookie"),
            ),
            text: Buffer.concat(chunks).toString("utf8"),
          }
        }
        return yield* failure("http-redirect-limit")
      })

      return SourceHttp.of({
        request: (input) =>
          request(input).pipe(
            Effect.timeout("30 seconds"),
            Effect.mapError((error) =>
              error instanceof ExtensionError
                ? error
                : failure("http-timeout", input.method !== "POST"),
            ),
            Effect.provideService(FetchHttpClient.RequestInit, { redirect: "manual" }),
            // Upstream URLs and headers can contain secrets. Emit our sanitized span above instead.
            Effect.provideService(HttpClient.TracerDisabledWhen, () => true),
          ),
      })
    }),
  ).pipe(Layer.provide(FetchHttpClient.layer))
