import type { Database } from "bun:sqlite"
import { openDatabase } from "./database/migrations"
import { sourceRepository } from "./database/sources"
import { jobRepository } from "./database/jobs"
import { publicationRepository } from "./database/publications"
import type { StoreOperations } from "./models"

declare const self: Worker

type Handlers = {
  [K in keyof StoreOperations]: (input: StoreOperations[K]["input"]) => StoreOperations[K]["output"]
}
type Request = {
  [K in keyof StoreOperations]: { id: number; action: K; input: StoreOperations[K]["input"] }
}[keyof StoreOperations]

let db: Database | undefined
let handlers: Handlers | undefined

function dispatch<K extends keyof StoreOperations>(action: K, input: StoreOperations[K]["input"]) {
  if (!handlers) throw new Error("Storage is not initialized")
  return handlers[action](input)
}

self.onmessage = (
  event: MessageEvent<
    Request | { id: number; action: "initialize"; input: string } | { id: number; action: "close" }
  >,
) => {
  const request = event.data

  try {
    if (request.action === "initialize") {
      if (db) throw new Error("Storage is already initialized")
      db = openDatabase(request.input)
      handlers = { ...sourceRepository(db), ...jobRepository(db), ...publicationRepository(db) }
      self.postMessage({ id: request.id, value: null })
      return
    }

    if (request.action === "close") {
      db?.close()
      self.postMessage({ id: request.id, value: null })
      return
    }

    self.postMessage({ id: request.id, value: dispatch(request.action, request.input) })
  } catch (error) {
    // SQL and stored credentials never cross the worker boundary.
    const code = (error as NodeJS.ErrnoException).code
    self.postMessage({
      id: request.id,
      error: "Storage operation failed",
      code: typeof code === "string" ? code : "DATABASE_FAILURE",
    })
  }
}
