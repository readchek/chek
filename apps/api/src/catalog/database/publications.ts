import type { Database } from "bun:sqlite"
import { Schema } from "effect"
import { Publication, type SourceAsset, type StoreOperations } from "../models"

type PublicationRow = Omit<Publication, "authors" | "metadata"> & {
  authors: string
  metadata: string
}
const decodePublication = Schema.decodeUnknownSync(Publication)

function fromRow(row: PublicationRow): Publication {
  return decodePublication({
    ...row,
    authors: JSON.parse(row.authors),
    metadata: JSON.parse(row.metadata),
  })
}

export function publicationRepository(db: Database) {
  return {
    publications: (query: StoreOperations["publications"]["input"]) => {
      const escaped = query.search.replace(/[\\%_]/g, "\\$&")
      const rows = query.search
        ? db
            .query<PublicationRow, [string, string, number]>(
              "SELECT * FROM publications WHERE id > ? AND title LIKE ? ESCAPE '\\' ORDER BY id LIMIT ?",
            )
            .all(query.after, `%${escaped}%`, query.limit)
        : db
            .query<PublicationRow, [string, number]>(
              "SELECT * FROM publications WHERE id > ? ORDER BY id LIMIT ?",
            )
            .all(query.after, query.limit)

      return rows.map(fromRow)
    },

    publication: (id: string) => {
      const row = db
        .query<PublicationRow, [string]>("SELECT * FROM publications WHERE id=?")
        .get(id)
      return row ? fromRow(row) : null
    },

    asset: (hash: string) =>
      db
        .query<{ hash: string; byteLength: number }, [string]>("SELECT * FROM assets WHERE hash=?")
        .get(hash),

    sourceAsset: (value: StoreOperations["sourceAsset"]["input"]) =>
      db
        .query<SourceAsset, [string, string]>(
          "SELECT assetHash, byteLength, sourceFingerprint FROM publications WHERE sourceId=? AND sourceKey=?",
        )
        .get(value.sourceId, value.sourceKey),

    import: (value: StoreOperations["import"]["input"]): "imported" | "skipped" =>
      db
        .transaction(() => {
          const active = db
            .query<{ state: string; attempts: number }, [string]>(
              "SELECT state, attempts FROM jobs WHERE id=?",
            )
            .get(value.jobId)

          if (active?.state !== "running" || active.attempts !== value.attempt)
            throw new Error("Job lease lost")

          const publication = value.publication
          const previous = db
            .query<{ id: string; assetHash: string }, [string, string]>(
              "SELECT id, assetHash FROM publications WHERE sourceId=? AND sourceKey=?",
            )
            .get(publication.sourceId, publication.sourceKey)
          const unchanged = previous?.assetHash === publication.assetHash
          const now = Date.now()

          db.query("INSERT OR IGNORE INTO assets VALUES (?, ?)").run(
            publication.assetHash,
            publication.byteLength,
          )
          db.query(`
        INSERT INTO publications (
          id, sourceId, sourceKey, title, format, mediaType, authors, language,
          assetHash, byteLength, createdAt, updatedAt, sourceFingerprint, metadata
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        ON CONFLICT(sourceId, sourceKey) DO UPDATE SET
          title=excluded.title, format=excluded.format, mediaType=excluded.mediaType,
          authors=excluded.authors, language=excluded.language, metadata=excluded.metadata,
          assetHash=excluded.assetHash, byteLength=excluded.byteLength,
          sourceFingerprint=excluded.sourceFingerprint, updatedAt=excluded.updatedAt
      `).run(
            previous?.id ?? crypto.randomUUID(),
            publication.sourceId,
            publication.sourceKey,
            publication.title,
            publication.format,
            publication.mediaType,
            JSON.stringify(publication.authors),
            publication.language,
            publication.assetHash,
            publication.byteLength,
            now,
            now,
            publication.sourceFingerprint,
            JSON.stringify(publication.metadata),
          )

          db.query(
            unchanged
              ? "UPDATE jobs SET skipped=skipped+1 WHERE id=?"
              : "UPDATE jobs SET imported=imported+1 WHERE id=?",
          ).run(value.jobId)

          return unchanged ? "skipped" : "imported"
        })
        .immediate(),
  }
}
