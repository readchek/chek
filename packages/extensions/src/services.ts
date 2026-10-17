import { Context, Effect, Schema } from "effect"
import { ExtensionError } from "./errors.js"

/** Host-managed text/JSON requests. Page bytes travel through core asset delivery instead. */
export interface SourceHttpRequest {
  readonly url: string
  readonly method?: "GET" | "POST"
  readonly headers?: Readonly<Record<string, string>>
  readonly body?: string
}

export interface SourceHttpResponse {
  readonly status: number
  readonly url: string
  readonly headers: Readonly<Record<string, string>>
  readonly text: string
  readonly byteLength: number
}

/**
 * Fetch text/JSON/HTML through the host. Responses are capped at 4 MiB and 30 seconds.
 * Cookies persist per source and origin. GET transport/429/5xx failures are retryable;
 * other statuses are returned for the extension to interpret. Never log credentials.
 */
export class SourceHttp extends Context.Service<
  SourceHttp,
  {
    readonly request: (
      request: SourceHttpRequest,
    ) => Effect.Effect<SourceHttpResponse, ExtensionError>
  }
>()("@chek/extensions/SourceHttp") {}

/**
 * Encrypted JSON state, isolated per configured source (64 KiB/value, 1,024 entries).
 * Validation uses temporary state. Get/set are not a transaction. For durable import
 * cursors, emit discovery checkpoints instead so progress cannot outrun ingestion.
 */
export class SourceState extends Context.Service<
  SourceState,
  {
    readonly get: (key: string) => Effect.Effect<Schema.Json | null, ExtensionError>
    readonly set: (key: string, value: Schema.Json) => Effect.Effect<void, ExtensionError>
    readonly remove: (key: string) => Effect.Effect<void, ExtensionError>
  }
>()("@chek/extensions/SourceState") {}
