import type { Database } from "bun:sqlite"
import { Job, type JobEvent, type StoreOperations } from "../models"
import { Schema } from "effect"

const decodeJob = Schema.decodeUnknownSync(Job)
const leaseMs = 30000
const maxAttempts = 3
const maxEvents = 1000

export function jobRepository(db: Database) {
  const job = (id: string) => {
    const row = db.query("SELECT * FROM jobs WHERE id=?").get(id)
    return row ? decodeJob(row) : null
  }

  const enqueue = (sourceId: string): Job =>
    db
      .transaction(() => {
        const active = db
          .query("SELECT * FROM jobs WHERE sourceId=? AND state IN ('queued','running')")
          .get(sourceId)
        if (active) return decodeJob(active)

        const id = crypto.randomUUID()
        const now = Date.now()
        db.query(
          "INSERT INTO jobs(id, sourceId, state, createdAt, updatedAt) VALUES (?, ?, 'queued', ?, ?)",
        ).run(id, sourceId, now, now)
        return job(id)!
      })
      .immediate()

  return {
    job,
    enqueue,
    jobs: () =>
      db
        .query("SELECT * FROM jobs ORDER BY createdAt DESC, id DESC LIMIT 100")
        .all()
        .map((row) => decodeJob(row)),
    events: (id: string) =>
      db
        .query<JobEvent, [string]>(
          "SELECT * FROM job_events WHERE jobId=? ORDER BY id DESC LIMIT 100",
        )
        .all(id),

    cancel: (id: string) => {
      db.query(
        "UPDATE jobs SET state='cancelled', updatedAt=?, leaseUntil=0 WHERE id=? AND state IN ('queued','running')",
      ).run(Date.now(), id)
      return job(id)
    },

    retry: (id: string) =>
      db
        .transaction(() => {
          const previous = job(id)
          if (!previous || !["failed", "cancelled"].includes(previous.state)) return previous
          return enqueue(previous.sourceId)
        })
        .immediate(),

    claim: () =>
      db
        .transaction(() => {
          const now = Date.now()

          db.query(`
        UPDATE jobs SET state='failed', errorCode='worker-retries-exhausted',
          errorMessage='Worker stopped repeatedly; retry the scan.', updatedAt=?
        WHERE state='running' AND leaseUntil < ? AND attempts >= ?
      `).run(now, now, maxAttempts)
          db.query(`
        UPDATE jobs SET state='queued', updatedAt=?
        WHERE state='running' AND leaseUntil < ? AND attempts < ?
      `).run(now, now, maxAttempts)

          const next = db
            .query<{ id: string }, [number]>(
              "SELECT id FROM jobs WHERE state='queued' AND nextAttemptAt<=? ORDER BY createdAt, id LIMIT 1",
            )
            .get(now)
          if (!next) return null

          db.query(
            "UPDATE jobs SET state='running', attempts=attempts+1, leaseUntil=?, updatedAt=? WHERE id=?",
          ).run(now + leaseMs, now, next.id)
          return job(next.id)
        })
        .immediate(),

    heartbeat: (value: StoreOperations["heartbeat"]["input"]) => {
      const now = Date.now()
      return (
        db
          .query(`
        UPDATE jobs SET leaseUntil=?, updatedAt=? WHERE id=? AND attempts=? AND state='running'
      `)
          .run(now + leaseMs, now, value.id, value.attempt).changes > 0
      )
    },

    trace: (value: StoreOperations["trace"]["input"]) => {
      db.query("UPDATE jobs SET traceId=? WHERE id=? AND attempts=? AND state='running'").run(
        value.traceId,
        value.id,
        value.attempt,
      )
      return null
    },

    finish: (value: StoreOperations["finish"]["input"]) => {
      const retry = Boolean(value.errorCode && value.retryable && value.attempt < maxAttempts)
      db.query(`
        UPDATE jobs SET state=?, errorCode=?, errorMessage=?, updatedAt=?, leaseUntil=0, nextAttemptAt=?
        WHERE id=? AND attempts=? AND state='running'
      `).run(
        retry ? "queued" : value.errorCode ? "failed" : "completed",
        value.errorCode,
        value.errorMessage,
        Date.now(),
        retry ? Date.now() + Math.min(60_000, 1000 * 2 ** value.attempt) : 0,
        value.id,
        value.attempt,
      )
      return null
    },

    checkpoint: (value: StoreOperations["checkpoint"]["input"]) => {
      db.query(
        `UPDATE sources SET checkpoint=? WHERE id=(SELECT sourceId FROM jobs WHERE id=? AND attempts=? AND state='running' AND leaseUntil>=?)`,
      ).run(value.cursor, value.id, value.attempt, Date.now())
      return null
    },

    event: (value: StoreOperations["event"]["input"]) =>
      db
        .transaction(() => {
          const active = job(value.id)
          if (active?.state !== "running" || active.attempts !== value.attempt) return null

          db.query("UPDATE jobs SET warnings=warnings+1 WHERE id=?").run(value.id)
          if (active.warnings < maxEvents) {
            db.query(
              "INSERT INTO job_events(jobId, code, message, createdAt) VALUES (?, ?, ?, ?)",
            ).run(value.id, value.code, value.message, Date.now())
          }

          return null
        })
        .immediate(),
  }
}
