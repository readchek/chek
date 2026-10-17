import { BunRuntime } from "@effect/platform-bun"
import {
  ExtensionContext,
  ExtensionError,
  ExtensionEvent,
  ExtensionRequest,
  protocolLimits,
  SourceState,
  type Extension,
} from "@chek/extensions"
import { Effect, Schema, Stream, Tracer } from "effect"
import { pathToFileURL } from "node:url"
import { TelemetryLive } from "../observability/telemetry"
import { ApiConfig } from "../services/config"
import { SessionStore, stateKey } from "./state"
import { sourceHttpLayer, cookieHeader } from "./http"
import type { AssetReference, SourceResult } from "@chek/core"

/** Attach only matching-origin cookies to private references before sending them to the host. */
function sessionAssets(result: SourceResult, store: SessionStore): SourceResult {
  const asset = (reference: AssetReference): AssetReference => {
    if (reference.kind === "file") return reference
    const cookie = cookieHeader(store, new URL(reference.url))
    return cookie ? { ...reference, headers: { cookie, ...reference.headers } } : reference
  }
  const series = (value: Extract<SourceResult, { operation: "series" }>["data"]) => ({
    ...value,
    ...(value.cover ? { cover: asset(value.cover) } : {}),
  })
  switch (result.operation) {
    case "series":
      return { ...result, data: series(result.data) }
    case "browse":
    case "search":
      return { ...result, data: { ...result.data, items: result.data.items.map(series) } }
    case "pages":
      return {
        ...result,
        data: {
          ...result.data,
          items: result.data.items.map((page) => ({ ...page, asset: asset(page.asset) })),
        },
      }
    case "chapters":
      return result
  }
}

// Only protocol records belong on stdout. Extension console output stays diagnostic.
console.log = console.error
console.info = console.error
console.debug = console.error

const transportError = () =>
  new ExtensionError({
    code: "extension-transport",
    message: "Extension transport failed",
    retryable: false,
  })

const writeEvent = (output: ReturnType<typeof Bun.stdout.writer>, event: ExtensionEvent) =>
  Effect.tryPromise({
    try: async () => {
      const line =
        JSON.stringify(
          Schema.decodeUnknownSync(ExtensionEvent)(event, { onExcessProperty: "error" }),
        ) + "\n"
      if (Buffer.byteLength(line) > protocolLimits.eventBytes) throw new Error("Event too large")
      output.write(line)
      await output.flush()
    },
    catch: transportError,
  })

const program = Effect.scoped(
  Effect.gen(function* () {
    const output = yield* Effect.acquireRelease(
      Effect.sync(() => Bun.stdout.writer()),
      (writer) =>
        Effect.promise(async () => {
          await writer.end()
        }),
    )

    yield* Effect.gen(function* () {
      const request = yield* Effect.tryPromise({
        try: async () => {
          let bytes = 0
          const chunks: Uint8Array[] = []
          for await (const chunk of Bun.stdin.stream()) {
            bytes += chunk.byteLength
            if (bytes > protocolLimits.requestBytes) throw new Error("Request too large")
            chunks.push(chunk)
          }
          return Schema.decodeUnknownSync(ExtensionRequest)(
            JSON.parse(Buffer.concat(chunks).toString("utf8")),
            { onExcessProperty: "error" },
          )
        },
        catch: transportError,
      })

      const extension = yield* Effect.tryPromise({
        try: async () => {
          const entrypoint = process.argv[2]
          if (!entrypoint) throw new Error("Missing entrypoint")
          const module: { default: Extension } = await import(pathToFileURL(entrypoint).href)
          const extension = module.default
          if (
            extension.manifest.id !== request.extensionId ||
            extension.manifest.version !== request.extensionVersion ||
            extension.manifest.protocolVersion !== request.protocolVersion
          ) {
            throw new Error("Manifest and executable disagree")
          }
          return extension
        },
        catch: () =>
          new ExtensionError({
            code: "extension-load",
            message: "Extension module does not match its installed manifest",
            retryable: false,
          }),
      })

      const sessions = yield* Effect.acquireRelease(
        Effect.tryPromise({
          try: async () => {
            const directory = process.argv[3]
            if (!directory) throw new Error("Missing runtime directory")
            return new SessionStore(
              directory,
              await stateKey(directory),
              request.sourceId ? `${request.extensionId}:${request.sourceId}` : undefined,
            )
          },
          catch: transportError,
        }),
        (store) => Effect.sync(() => store.close()),
      )
      const execution =
        request.operation === "validate"
          ? extension.validate(request.config)
          : request.operation === "query"
            ? extension
                .query(request.config, request.query)
                .pipe(
                  Effect.flatMap((result) =>
                    Effect.try({
                      try: () => sessionAssets(result, sessions),
                      catch: transportError,
                    }).pipe(
                      Effect.flatMap((result) => writeEvent(output, { type: "result", result })),
                    ),
                  ),
                )
            : Stream.runForEach(extension.discover(request.config), (event) =>
                writeEvent(output, event),
              )

      const traced = execution.pipe(
        Effect.provideService(ExtensionContext, { grants: request.grants }),
        Effect.provideService(SourceState, sessions.service),
        Effect.provide(sourceHttpLayer(sessions, request.grants)),
        Effect.tapError((error) =>
          Effect.logError("Extension operation failed", {
            extensionId: request.extensionId,
            operation: request.operation,
            code: error.code,
            retryable: error.retryable,
          }),
        ),
        Effect.withSpan(`extension.${request.operation}`, {
          attributes: {
            "chek.extension.id": request.extensionId,
            "chek.extension.version": request.extensionVersion,
          },
        }),
      )
      yield* request.trace
        ? traced.pipe(Effect.withParentSpan(Tracer.externalSpan(request.trace)))
        : traced
      yield* writeEvent(output, { type: "complete" })
    }).pipe(
      Effect.catch((error) =>
        writeEvent(output, {
          type: "failure",
          code: error.code,
          message: error.message,
          retryable: error.retryable,
        }),
      ),
    )
  }),
)

BunRuntime.runMain(program.pipe(Effect.provide(TelemetryLive), Effect.provide(ApiConfig.layer)))
