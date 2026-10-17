import { Effect, Layer, Metric, Schedule, Stream } from "effect"
import { CatalogStore } from "../catalog/store"
import { PublicationIngestion } from "../catalog/ingestion"
import { type Job } from "../catalog/models"
import { ExtensionRegistry } from "../extensions/registry"
import { ExtensionHost } from "../extensions/host"
import { ApiConfig } from "../services/config"
import { ImportWakeup } from "./wakeup"

const failures = Metric.counter("chek.import.failures")
const duration = Metric.histogram("chek.import.duration_ms", {
  boundaries: [100, 500, 1000, 5000, 15000, 60000, 300000, 3600000],
})

const runJob = Effect.fn("imports.scan")(function* (job: Job) {
  const store = yield* CatalogStore
  const registry = yield* ExtensionRegistry
  const host = yield* ExtensionHost
  const ingestion = yield* PublicationIngestion
  const started = performance.now()

  yield* Effect.annotateCurrentSpan({
    "chek.job.id": job.id,
    "chek.source.id": job.sourceId,
    "chek.job.attempt": job.attempts,
  })
  const span = yield* Effect.currentSpan
  yield* store.call("trace", { id: job.id, attempt: job.attempts, traceId: span.traceId })

  const process = Effect.scoped(
    Effect.gen(function* () {
      const source = yield* store.call("source", job.sourceId)
      const extension = source && registry.installed.get(source.extensionId)

      if (!source || !extension) {
        yield* store.call("finish", {
          id: job.id,
          attempt: job.attempts,
          errorCode: "extension-missing",
          errorMessage: "Source extension is no longer installed",
        })
        return
      }

      const controller = yield* Effect.acquireRelease(
        Effect.sync(() => new AbortController()),
        (value) => Effect.sync(() => value.abort()),
      )
      const heartbeat = Effect.gen(function* () {
        const active = yield* store.call("heartbeat", { id: job.id, attempt: job.attempts })
        if (!active) controller.abort()
      }).pipe(Effect.catch(() => Effect.sync(() => controller.abort())))

      yield* heartbeat.pipe(Effect.repeat(Schedule.spaced("5 seconds")), Effect.forkScoped)
      yield* Effect.logInfo("Import started", {
        jobId: job.id,
        sourceId: job.sourceId,
        extensionId: source.extensionId,
        attempt: job.attempts,
      })

      const events = host.execute(
        extension,
        {
          protocolVersion: 1,
          operation: "discover",
          sourceId: source.id,
          checkpoint: source.checkpoint,
          extensionId: extension.manifest.id,
          extensionVersion: extension.manifest.version,
          config: source.config,
          grants: { readRoots: source.readRoots, httpOrigins: source.httpOrigins },
        },
        controller.signal,
      )

      // Never move a resume cursor past an asset that failed to import.
      let checkpointSafe = true
      yield* Stream.runForEach(events, (event) =>
        Effect.gen(function* () {
          if (event.type === "complete" || event.type === "failure" || event.type === "result")
            return

          if (event.type === "checkpoint") {
            if (checkpointSafe)
              yield* store.call("checkpoint", {
                id: job.id,
                attempt: job.attempts,
                cursor: event.cursor,
              })
            return
          }

          if (event.type === "warning") {
            yield* store.call("event", {
              id: job.id,
              attempt: job.attempts,
              code: event.code,
              message: "Extension reported a discovery warning",
            })
            yield* Effect.logWarning("Discovery warning", { jobId: job.id, code: event.code })
            return
          }

          yield* ingestion
            .ingest(event.publication, { job, grants: source, signal: controller.signal })
            .pipe(
              Effect.catchTag("RequestError", (error) =>
                Effect.gen(function* () {
                  checkpointSafe = false
                  yield* Metric.update(failures, 1)
                  yield* store.call("event", {
                    id: job.id,
                    attempt: job.attempts,
                    code: error.code,
                    message: error.message,
                  })
                  yield* Effect.logWarning("Asset import failed", {
                    jobId: job.id,
                    code: error.code,
                  })

                  if (["storage-full", "asset-integrity"].includes(error.code)) return yield* error
                }),
              ),
            )
        }),
      )

      yield* store.call("finish", {
        id: job.id,
        attempt: job.attempts,
        errorCode: null,
        errorMessage: null,
      })
      const result = yield* store.call("job", job.id)
      yield* Effect.logInfo("Import finished", {
        jobId: job.id,
        state: result?.state,
        imported: result?.imported,
        skipped: result?.skipped,
        warnings: result?.warnings,
      })
    }),
  )

  yield* process.pipe(
    Effect.catch((error) =>
      Effect.gen(function* () {
        yield* Metric.update(failures, 1)
        yield* store.call("finish", {
          id: job.id,
          attempt: job.attempts,
          errorCode: error.code,
          errorMessage: error.message,
          retryable: "retryable" in error && error.retryable === true,
        })
        yield* Effect.logError("Import failed", { jobId: job.id, code: error.code })
      }),
    ),
    Effect.ensuring(Effect.suspend(() => Metric.update(duration, performance.now() - started))),
  )
})

export const ImportWorkerLive = Layer.effectDiscard(
  Effect.gen(function* () {
    const store = yield* CatalogStore
    const config = yield* ApiConfig
    const wakeup = yield* ImportWakeup
    const poll = Effect.gen(function* () {
      const job = yield* store.call("claim", null)
      if (job) yield* runJob(job)
      else yield* wakeup.wait
    }).pipe(
      Effect.catchCause((cause) =>
        Effect.gen(function* () {
          yield* Effect.logError("Import worker unavailable; retrying", cause)
          yield* Effect.sleep("5 seconds")
        }),
      ),
    )

    for (let worker = 0; worker < config.importConcurrency; worker++) {
      yield* poll.pipe(Effect.forever, Effect.forkScoped)
    }
    yield* Effect.logInfo("Import worker started", {
      concurrency: config.importConcurrency,
      leaseSeconds: 30,
    })
  }),
)
