import { BunRuntime } from "@effect/platform-bun"
import {
  ExtensionContext,
  ExtensionError,
  ExtensionEvent,
  ExtensionRequest,
  protocolLimits,
  type Extension,
} from "@chek/extensions"
import { Effect, Schema, Stream, Tracer } from "effect"
import { pathToFileURL } from "node:url"
import { TelemetryLive } from "../observability/telemetry"
import { ApiConfig } from "../services/config"

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

      const execution =
        request.operation === "validate"
          ? extension.validate(request.config)
          : Stream.runForEach(extension.discover(request.config), (event) =>
              writeEvent(output, event),
            )

      const traced = execution.pipe(
        Effect.provideService(ExtensionContext, { grants: request.grants }),
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
