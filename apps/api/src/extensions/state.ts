import { Database } from "bun:sqlite"
import { createCipheriv, createDecipheriv, randomBytes } from "node:crypto"
import { mkdir, readFile, open } from "node:fs/promises"
import { join } from "node:path"
import { Effect, Schema } from "effect"
import { ExtensionError, SourceState } from "@chek/extensions"

/** The key lives outside SQLite so database backups do not contain session credentials. */
export async function stateKey(directory: string): Promise<Buffer> {
  await mkdir(directory, { recursive: true, mode: 0o700 })
  const path = join(directory, "sessions.key")
  try {
    const file = await open(path, "wx", 0o600)
    try {
      await file.writeFile(randomBytes(32))
      await file.sync()
    } finally {
      await file.close()
    }
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code !== "EEXIST") throw error
  }
  const key = await readFile(path)
  if (key.length !== 32) throw new Error("Invalid session encryption key")
  return key
}

/** Runs only in the extension subprocess. SQLite transactions coordinate concurrent invocations. */
export class SessionStore {
  private readonly db: Database

  constructor(
    directory: string,
    private readonly key: Buffer,
    private readonly namespace: string | undefined,
  ) {
    this.db = new Database(namespace ? join(directory, "sessions.sqlite") : ":memory:", {
      create: true,
    })
    this.db.exec("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA busy_timeout=5000")
    this.db.exec(
      "CREATE TABLE IF NOT EXISTS state(namespace TEXT, key TEXT, value BLOB NOT NULL, PRIMARY KEY(namespace,key)); CREATE TABLE IF NOT EXISTS pacing(origin TEXT PRIMARY KEY, nextAt INTEGER NOT NULL)",
    )
  }

  close() {
    this.db.close()
  }

  get(name: string): Schema.Json | null {
    const row = this.db
      .query<{ value: Uint8Array }, [string, string]>(
        "SELECT value FROM state WHERE namespace=? AND key=?",
      )
      .get(this.namespace ?? "validation", name)
    if (!row) return null
    const bytes = Buffer.from(row.value)
    const cipher = createDecipheriv("aes-256-gcm", this.key, bytes.subarray(0, 12))
    cipher.setAAD(Buffer.from(`${this.namespace ?? "validation"}:${name}`))
    cipher.setAuthTag(bytes.subarray(12, 28))
    return Schema.decodeUnknownSync(Schema.Json)(
      JSON.parse(Buffer.concat([cipher.update(bytes.subarray(28)), cipher.final()]).toString()),
    )
  }

  set(name: string, value: Schema.Json) {
    if (!name || name.length > 2048) throw new Error("Invalid state key")
    const text = JSON.stringify(Schema.decodeUnknownSync(Schema.Json)(value))
    if (Buffer.byteLength(text) > 65536) throw new Error("State value exceeds 64 KiB")
    const iv = randomBytes(12)
    const cipher = createCipheriv("aes-256-gcm", this.key, iv)
    cipher.setAAD(Buffer.from(`${this.namespace ?? "validation"}:${name}`))
    const encrypted = Buffer.concat([cipher.update(text), cipher.final()])
    this.db
      .transaction(() => {
        const count = this.db
          .query<{ count: number }, [string]>(
            "SELECT count(*) AS count FROM state WHERE namespace=?",
          )
          .get(this.namespace ?? "validation")!.count
        if (count >= 1024 && this.get(name) === null) throw new Error("Source state limit reached")
        this.db
          .query("INSERT OR REPLACE INTO state VALUES (?,?,?)")
          .run(
            this.namespace ?? "validation",
            name,
            Buffer.concat([iv, cipher.getAuthTag(), encrypted]),
          )
      })
      .immediate()
  }

  remove(name: string) {
    this.db
      .query("DELETE FROM state WHERE namespace=? AND key=?")
      .run(this.namespace ?? "validation", name)
  }

  update(name: string, change: (previous: Schema.Json | null) => Schema.Json) {
    this.db.transaction(() => this.set(name, change(this.get(name)))).immediate()
  }

  /** Reserve an origin-wide request slot; never hold a transaction while sleeping. */
  reserve(origin: string, delayMs = 250): number {
    return this.db
      .transaction(() => {
        const row = this.db
          .query<{ nextAt: number }, [string]>("SELECT nextAt FROM pacing WHERE origin=?")
          .get(origin)
        const now = Date.now()
        const at = Math.max(now, row?.nextAt ?? now)
        this.db.query("INSERT OR REPLACE INTO pacing VALUES (?,?)").run(origin, at + delayMs)
        return at - now
      })
      .immediate()
  }

  readonly service = SourceState.of({
    get: (key) => this.operation(() => this.get(`extension:${key}`)),
    set: (key, value) => this.operation(() => this.set(`extension:${key}`, value)),
    remove: (key) => this.operation(() => this.remove(`extension:${key}`)),
  })

  private operation<A>(run: () => A) {
    return Effect.try({
      try: run,
      catch: () =>
        new ExtensionError({
          code: "source-state",
          message: "Source state could not be accessed",
          retryable: false,
        }),
    })
  }
}
