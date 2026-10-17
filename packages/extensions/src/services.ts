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
}

/** Enforces grants, bounded bodies, timeouts, origin-scoped cookies, and shared request pacing. */
export class SourceHttp extends Context.Service<
  SourceHttp,
  {
    readonly request: (
      request: SourceHttpRequest,
    ) => Effect.Effect<SourceHttpResponse, ExtensionError>
  }
>()("@chek/extensions/SourceHttp") {}

/** Durable, encrypted, source-scoped JSON state. Use for tokens and source-specific cursors. */
export class SourceState extends Context.Service<
  SourceState,
  {
    readonly get: (key: string) => Effect.Effect<Schema.Json | null, ExtensionError>
    readonly set: (key: string, value: Schema.Json) => Effect.Effect<void, ExtensionError>
    readonly remove: (key: string) => Effect.Effect<void, ExtensionError>
  }
>()("@chek/extensions/SourceState") {}
