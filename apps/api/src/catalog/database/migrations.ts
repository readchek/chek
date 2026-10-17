import { Database } from "bun:sqlite"
import { chmodSync, mkdirSync } from "node:fs"
import { join } from "node:path"

export const schemaVersion = 3

const migrations = [
  `
    CREATE TABLE sources (
      id TEXT PRIMARY KEY, name TEXT NOT NULL, extensionId TEXT NOT NULL,
      config TEXT NOT NULL, readRoots TEXT NOT NULL, httpOrigins TEXT NOT NULL,
      createdAt INTEGER NOT NULL
    );
    CREATE TABLE jobs (
      id TEXT PRIMARY KEY, sourceId TEXT NOT NULL REFERENCES sources(id),
      state TEXT NOT NULL CHECK(state IN ('queued','running','completed','failed','cancelled')),
      attempts INTEGER NOT NULL DEFAULT 0, imported INTEGER NOT NULL DEFAULT 0,
      skipped INTEGER NOT NULL DEFAULT 0, warnings INTEGER NOT NULL DEFAULT 0,
      createdAt INTEGER NOT NULL, updatedAt INTEGER NOT NULL,
      leaseUntil INTEGER NOT NULL DEFAULT 0, errorCode TEXT, errorMessage TEXT, traceId TEXT
    );
    CREATE UNIQUE INDEX jobs_active_source ON jobs(sourceId) WHERE state IN ('queued','running');
    CREATE INDEX jobs_queue ON jobs(state, createdAt);
    CREATE TABLE job_events (
      id INTEGER PRIMARY KEY, jobId TEXT NOT NULL REFERENCES jobs(id),
      code TEXT NOT NULL, message TEXT NOT NULL, createdAt INTEGER NOT NULL
    );
    CREATE INDEX job_events_job ON job_events(jobId, id);
    CREATE TABLE assets (hash TEXT PRIMARY KEY, byteLength INTEGER NOT NULL);
    CREATE TABLE publications (
      id TEXT PRIMARY KEY, sourceId TEXT NOT NULL REFERENCES sources(id),
      sourceKey TEXT NOT NULL, title TEXT NOT NULL, format TEXT NOT NULL, mediaType TEXT NOT NULL,
      authors TEXT NOT NULL, language TEXT, assetHash TEXT NOT NULL REFERENCES assets(hash),
      byteLength INTEGER NOT NULL, status TEXT NOT NULL DEFAULT 'stored',
      createdAt INTEGER NOT NULL, updatedAt INTEGER NOT NULL, UNIQUE(sourceId, sourceKey)
    );
    CREATE INDEX publications_title ON publications(title COLLATE NOCASE, id);
  `,
  "ALTER TABLE publications ADD COLUMN sourceFingerprint TEXT;",
  "ALTER TABLE publications ADD COLUMN metadata TEXT NOT NULL DEFAULT '{}';",
] as const

export function openDatabase(directory: string): Database {
  mkdirSync(directory, { recursive: true, mode: 0o700 })
  const path = join(directory, "catalog.sqlite")
  const db = new Database(path, { create: true, strict: true })

  try {
    chmodSync(path, 0o600)
    db.exec(
      "PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000;",
    )
    const version = db
      .query<{ user_version: number }, []>("PRAGMA user_version")
      .get()!.user_version
    if (version > schemaVersion) throw new Error("Database belongs to a newer Chek version")

    migrations.forEach((sql, index) => {
      if (index < version) return
      db.transaction(() => {
        db.exec(sql)
        db.exec(`PRAGMA user_version=${index + 1}`)
      }).immediate()
    })

    return db
  } catch (error) {
    db.close()
    throw error
  }
}
