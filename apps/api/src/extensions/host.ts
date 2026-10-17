import { ExtensionError, type ExtensionEvent, type ExtensionRequest } from "@chek/extensions"
import { Context, Effect, Layer, Stream } from "effect"
import { ApiConfig } from "../services/config"
import { discover, ExtensionFailure } from "./process"
import type { InstalledExtension } from "./installation"

export class ExtensionHost extends Context.Service<
  ExtensionHost,
  {
    readonly execute: (
      extension: InstalledExtension,
      request: ExtensionRequest,
      signal?: AbortSignal,
    ) => Stream.Stream<ExtensionEvent, ExtensionError>
  }
>()("@chek/api/ExtensionHost") {
  static readonly layer = Layer.effect(
    ExtensionHost,
    Effect.gen(function* () {
      const config = yield* ApiConfig

      return ExtensionHost.of({
        execute: (extension, request, signal) =>
          Stream.unwrap(
            Effect.gen(function* () {
              const controller = yield* Effect.acquireRelease(
                Effect.sync(() => new AbortController()),
                (value) => Effect.sync(() => value.abort()),
              )
              const cancellation = signal
                ? AbortSignal.any([signal, controller.signal])
                : controller.signal
              const timeout = request.operation === "validate" ? 15000 : config.extensionTimeoutMs
              const span = yield* Effect.currentSpan.pipe(Effect.orElseSucceed(() => undefined))
              const tracedRequest = {
                ...request,
                trace: span ? { traceId: span.traceId, spanId: span.spanId } : undefined,
              }

              return Stream.fromAsyncIterable(
                discover(extension, tracedRequest, timeout, cancellation),
                (error) =>
                  new ExtensionError({
                    code: error instanceof ExtensionFailure ? error.code : "extension-interrupted",
                    message:
                      error instanceof ExtensionFailure
                        ? error.message
                        : "Extension execution was interrupted",
                    retryable:
                      error instanceof ExtensionFailure &&
                      (error.retryable || error.code === "extension-timeout"),
                  }),
              )
            }),
          ),
      })
    }),
  )
}
