import { ExtensionEvent, type ExtensionRequest } from "@chek/extensions"
import { Schema } from "effect"
import { fileURLToPath } from "node:url"
import type { InstalledExtension } from "./installation"

export class ExtensionFailure extends Error {
  constructor(
    readonly code: string,
    message: string,
    readonly retryable = false,
  ) {
    super(message)
  }
}

export async function* discover(
  extension: InstalledExtension,
  request: ExtensionRequest,
  timeoutMs: number,
  signal: AbortSignal,
): AsyncGenerator<ExtensionEvent> {
  signal.throwIfAborted()
  const telemetryEnvironment = Object.fromEntries(
    Object.entries(Bun.env).filter(([name]) => name.startsWith("OTEL_") || name === "LOG_LEVEL"),
  )
  const process = Bun.spawn(
    [
      Bun.which("bun") ?? "bun",
      "--no-env-file",
      fileURLToPath(new URL("./runner.ts", import.meta.url)),
      extension.entrypoint,
    ],
    {
      stdin: new Blob([JSON.stringify(request)]),
      stdout: "pipe",
      stderr: "pipe",
      // Explicitly installed extensions are trusted code. This is crash containment, not an OS sandbox.
      env: {
        ...telemetryEnvironment,
        PATH: "/usr/bin:/bin:/usr/local/bin:/opt/homebrew/bin",
        LANG: "en_US.UTF-8",
        LOG_FORMAT: "json",
      },
    },
  )
  let timedOut = false
  let stderr = ""
  const decoder = new TextDecoder()
  const diagnostics = (async () => {
    for await (const chunk of process.stderr) {
      stderr = (stderr + decoder.decode(chunk, { stream: true })).slice(-8192)
    }
  })()
  const stop = () => process.kill("SIGKILL")
  const timeout = setTimeout(() => {
    timedOut = true
    stop()
  }, timeoutMs)
  signal.addEventListener("abort", stop, { once: true })
  let completed = false
  let failure: ExtensionFailure | undefined
  try {
    let pending = ""
    const decode = new TextDecoder("utf-8", { fatal: true })
    for await (const chunk of process.stdout) {
      signal.throwIfAborted()
      pending += decode.decode(chunk, { stream: true })
      let end: number
      while ((end = pending.indexOf("\n")) !== -1) {
        const line = pending.slice(0, end)
        pending = pending.slice(end + 1)
        if (Buffer.byteLength(line) > 65536)
          throw new ExtensionFailure("extension-output-limit", "Extension event exceeds 64 KiB")
        if (completed)
          throw new ExtensionFailure(
            "extension-protocol",
            "Extension emitted data after completion",
          )
        let event: ExtensionEvent
        try {
          event = Schema.decodeUnknownSync(ExtensionEvent)(JSON.parse(line))
        } catch {
          throw new ExtensionFailure(
            "extension-protocol",
            "Extension emitted an invalid protocol event",
          )
        }
        if (event.type === "failure") {
          failure = new ExtensionFailure(
            event.code,
            "Extension could not complete the operation",
            event.retryable,
          )
          completed = true
          continue
        }
        if (request.operation === "validate" && event.type !== "complete") {
          throw new ExtensionFailure("extension-protocol", "Validation emitted discovery data")
        }
        if (event.type === "complete") completed = true
        yield event
      }
      if (Buffer.byteLength(pending) > 65536)
        throw new ExtensionFailure("extension-output-limit", "Extension event exceeds 64 KiB")
    }
    const exitCode = await process.exited
    await diagnostics
    signal.throwIfAborted()
    if (timedOut)
      throw new ExtensionFailure("extension-timeout", "Extension exceeded its execution deadline")
    if (exitCode !== 0) {
      // Strip configured values, URL queries, and stack traces from user-facing diagnostics.
      let detail =
        stderr.split("\n").find((line) => /^(?:Error|error):/.test(line)) ??
        "Extension process failed"
      for (const value of Object.values(request.config))
        if (value) detail = detail.replaceAll(value, "[redacted]")
      detail = detail.replace(/https?:\/\/\S+/g, "[URL]").slice(0, 512)
      throw new ExtensionFailure("extension-exit", `${detail} (exit ${exitCode})`)
    }
    if (!completed || pending.length)
      throw new ExtensionFailure(
        "extension-incomplete",
        "Extension stopped without completing its protocol",
      )
    if (failure) throw failure
  } finally {
    clearTimeout(timeout)
    signal.removeEventListener("abort", stop)
    stop()
    await process.exited
    await diagnostics.catch(() => {})
  }
}
