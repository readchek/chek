import { ExtensionError, SourceHttp } from "@chek/extensions"
import { Effect, Schema } from "effect"
import * as Base64 from "effect/encoding/Base64"

export const Config = Schema.Struct({
  url: Schema.NonEmptyString,
  apiKey: Schema.optional(Schema.String),
  email: Schema.optional(Schema.String),
  password: Schema.optional(Schema.String),
  libraryId: Schema.optional(Schema.String),
})
export type Config = typeof Config.Type

export const komgaError = (code: string, message: string, retryable = false) =>
  new ExtensionError({ code, message, retryable })

/** Preserve reverse-proxy base paths; never follow upstream filesystem URLs from DTOs. */
export function endpoint(config: Config, path: string): string {
  return `${config.url.replace(/\/+$/, "")}/api/${path}`
}

/** Explicit headers also accompany private page/file references; they never reach the browser. */
export function authentication(config: Config): Readonly<Record<string, string>> {
  return config.apiKey
    ? { "x-api-key": config.apiKey }
    : { authorization: `Basic ${Base64.encode(`${config.email}:${config.password}`)}` }
}

export const validateConfig = Effect.fnUntraced(function* (config: Config) {
  const url = yield* Effect.try({
    try: () => new URL(config.url),
    catch: () => komgaError("komga-url", "Enter a valid Komga server URL"),
  })
  if (
    !["https:", "http:"].includes(url.protocol) ||
    url.username ||
    url.password ||
    url.search ||
    url.hash
  )
    return yield* komgaError(
      "komga-url",
      "Use an HTTP(S) server URL without embedded credentials, query or fragment",
    )
  if (!config.apiKey && (!config.email || !config.password || config.email.includes(":")))
    return yield* komgaError("komga-auth", "Provide an API key or email and password")
})

/** POST is used only for Komga's read-only list endpoints, so transient list failures are safe to retry. */
export const request = <S extends Schema.Top & { readonly DecodingServices: never }>(
  config: Config,
  path: string,
  schema: S,
  body?: unknown,
) =>
  Effect.gen(function* () {
    const http = yield* SourceHttp
    const response = yield* http
      .request({
        url: endpoint(config, path),
        method: body === undefined ? "GET" : "POST",
        headers: { ...authentication(config), accept: "application/json" },
        ...(body === undefined ? {} : { body: JSON.stringify(body) }),
      })
      .pipe(
        Effect.mapError((error) =>
          /^(http-network|http-timeout|http-429|http-5\d\d)$/.test(error.code)
            ? komgaError(error.code, "Komga is temporarily unavailable", true)
            : error,
        ),
      )
    if (response.status === 401)
      return yield* komgaError("komga-auth", "Komga rejected the credentials")
    if (response.status === 403)
      return yield* komgaError(
        "komga-permission",
        "Komga denied access; check the account's library and streaming/download permissions",
      )
    if (response.status === 404)
      return yield* komgaError("komga-missing", "This book, series or API endpoint is unavailable")
    if (response.status !== 200)
      return yield* komgaError(
        `komga-http-${response.status}`,
        "Komga could not complete the request",
      )
    return yield* Schema.decodeUnknownEffect(Schema.fromJsonString(schema))(response.text).pipe(
      Effect.mapError(() => komgaError("komga-response", "Komga returned an unexpected response")),
    )
  })

/** Cursor includes page size to prevent skipped books when callers change limits mid-list. */
export const cursorPage = Effect.fnUntraced(function* (cursor: string | undefined, limit: number) {
  if (!cursor) return 0
  const parsed = yield* Schema.decodeUnknownEffect(
    Schema.fromJsonString(
      Schema.Struct({ page: Schema.Int.check(Schema.isGreaterThanOrEqualTo(0)), size: Schema.Int }),
    ),
  )(cursor).pipe(
    Effect.mapError(() => komgaError("komga-cursor", "Invalid cursor; restart the list")),
  )
  if (parsed.size !== limit)
    return yield* komgaError("komga-cursor", "Keep the same page size when continuing a list")
  return parsed.page
})
export const nextCursor = (page: { last: boolean; number: number }, size: number) =>
  page.last ? null : JSON.stringify({ page: page.number + 1, size })
export const equal = (field: string, value: string) => ({ [field]: { operator: "is", value } })
export const visible = { deleted: { operator: "isFalse" } }

/** A configured library is an extension-level scope, including direct lookups by ID. */
export const checkLibrary = (config: Config, libraryId: string) =>
  config.libraryId && config.libraryId !== libraryId
    ? Effect.fail(komgaError("komga-library", "This item belongs to another library"))
    : Effect.void
