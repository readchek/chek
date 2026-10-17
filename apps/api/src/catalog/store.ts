import { Context, Effect, Layer } from "effect"
import { ApiConfig } from "../services/config"
import { StorageError, type StoreOperations } from "./models"

interface StoreService {
  call<K extends keyof StoreOperations>(
    action: K,
    input: StoreOperations[K]["input"],
  ): Effect.Effect<StoreOperations[K]["output"], StorageError>
}

export class CatalogStore extends Context.Service<CatalogStore, StoreService>()(
  "@chek/api/CatalogStore",
) {
  static readonly layer = Layer.effect(
    CatalogStore,
    Effect.gen(function* () {
      const config = yield* ApiConfig
      const worker = yield* Effect.acquireRelease(
        Effect.sync(() => new Worker(new URL("./database-worker.ts", import.meta.url).href)),
        (worker) => Effect.sync(() => worker.terminate()),
      )
      let sequence = 0
      let failed = false
      const pending = new Map<
        number,
        { resolve: (value: unknown) => void; reject: (reason: Error) => void }
      >()
      worker.onmessage = (event: MessageEvent) => {
        const response = event.data
        const waiter = pending.get(response.id)
        if (!waiter) return
        if (response.error)
          waiter.reject(Object.assign(new Error(response.error), { code: response.code }))
        else waiter.resolve(response.value)
      }
      worker.onerror = () => {
        failed = true
        for (const waiter of pending.values()) waiter.reject(new Error("Database worker stopped"))
      }
      function request(action: string, input: unknown) {
        const operation = Effect.tryPromise({
          try: (signal) =>
            new Promise<unknown>((resolve, reject) => {
              if (failed) {
                reject(new Error("Database worker unavailable"))
                return
              }
              const id = ++sequence
              const cleanup = () => {
                pending.delete(id)
                clearTimeout(timer)
                signal.removeEventListener("abort", abort)
              }
              const abort = () => {
                cleanup()
                reject(new Error("Database request cancelled"))
              }
              const timer = setTimeout(() => {
                cleanup()
                reject(new Error("Database request timed out"))
              }, 10000)
              pending.set(id, {
                resolve: (value) => {
                  cleanup()
                  resolve(value)
                },
                reject: (error) => {
                  cleanup()
                  reject(error)
                },
              })
              signal.addEventListener("abort", abort, { once: true })
              worker.postMessage({ id, action, input })
            }),
          catch: (error) =>
            new StorageError({
              message: "Catalog storage is unavailable",
              code: (error as NodeJS.ErrnoException).code ?? "DATABASE_UNAVAILABLE",
            }),
        }).pipe(
          Effect.tapError((error) =>
            Effect.logError("Catalog operation failed", { operation: action, code: error.code }),
          ),
        )

        // Empty polls and lease refreshes should not flood the trace collector.
        return action === "claim" || action === "heartbeat"
          ? operation
          : operation.pipe(Effect.withSpan(`catalog.${action}`))
      }
      yield* request("initialize", config.dataDirectory)
      yield* Effect.addFinalizer(() => request("close", null).pipe(Effect.ignore))
      yield* Effect.logInfo("Catalog storage ready", { schemaVersion: 3, journalMode: "wal" })
      return CatalogStore.of({
        call: <K extends keyof StoreOperations>(action: K, input: StoreOperations[K]["input"]) =>
          request(action, input) as Effect.Effect<StoreOperations[K]["output"], StorageError>,
      })
    }),
  )
}
