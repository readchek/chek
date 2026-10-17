import { Context, Effect, Layer, Queue } from "effect"
import { ApiConfig } from "../services/config"

/** Wake idle workers immediately; SQLite remains the durable queue. */
export class ImportWakeup extends Context.Service<
  ImportWakeup,
  {
    readonly notify: Effect.Effect<void>
    readonly wait: Effect.Effect<void>
  }
>()("@chek/api/ImportWakeup") {
  static readonly layer = Layer.effect(
    ImportWakeup,
    Effect.gen(function* () {
      const config = yield* ApiConfig
      const queue = yield* Queue.sliding<void>(config.importConcurrency)

      return ImportWakeup.of({
        notify: Queue.offer(queue, undefined).pipe(Effect.asVoid),
        wait: Queue.take(queue).pipe(Effect.timeoutOption("5 seconds"), Effect.asVoid),
      })
    }),
  )
}
