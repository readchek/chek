import type { Database } from "bun:sqlite"
import { Schema } from "effect"
import { Source, SourceInput, StoredSource } from "../models"

type SourceRow = Source & { config: string; readRoots: string; httpOrigins: string }
const decodeSource = Schema.decodeUnknownSync(StoredSource)

export function sourceRepository(db: Database) {
  return {
    sources: () =>
      db
        .query<Source, []>(
          "SELECT id, name, extensionId, createdAt FROM sources ORDER BY createdAt, id",
        )
        .all(),

    source: (id: string) => {
      const row = db.query<SourceRow, [string]>("SELECT * FROM sources WHERE id = ?").get(id)
      if (!row) return null

      return decodeSource({
        ...row,
        config: JSON.parse(row.config),
        readRoots: JSON.parse(row.readRoots),
        httpOrigins: JSON.parse(row.httpOrigins),
      })
    },

    createSource: (source: SourceInput): Source => {
      const id = crypto.randomUUID()
      const createdAt = Date.now()

      db.query("INSERT INTO sources VALUES (?, ?, ?, ?, ?, ?, ?)").run(
        id,
        source.name,
        source.extensionId,
        JSON.stringify(source.config),
        JSON.stringify(source.readRoots),
        JSON.stringify(source.httpOrigins),
        createdAt,
      )

      return { id, name: source.name, extensionId: source.extensionId, createdAt }
    },
  }
}
