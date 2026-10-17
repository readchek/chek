import { AssetReference } from "@chek/core"
import { Effect, Schedule, Schema } from "effect"
import { createCipheriv, createDecipheriv, createHmac, randomBytes } from "node:crypto"
import { mkdir, readdir, readFile, rename, unlink, writeFile } from "node:fs/promises"
import { join } from "node:path"
import { RequestError, StorageError } from "../catalog/models"

export const Resource = Schema.Struct({
  sourceId: Schema.String,
  asset: AssetReference,
  mediaType: Schema.String,
  expiresAt: Schema.Number,
})
export type Resource = typeof Resource.Type

/** Short opaque URLs reference encrypted records, including large upstream headers, across restarts. */
export const resourceStore = Effect.fnUntraced(function* (directory: string, key: Buffer) {
  const root = join(directory, "source-resources")
  yield* Effect.promise(() => mkdir(root, { recursive: true, mode: 0o700 }))
  const tokenPattern = /^(\d{13})-[a-f0-9]{64}$/

  const cleanup = Effect.tryPromise({
    try: async () => {
      for (const name of await readdir(root)) {
        const match = tokenPattern.exec(name)
        if (match && Number(match[1]) < Date.now()) await unlink(join(root, name)).catch(() => {})
      }
    },
    catch: () =>
      new StorageError({
        code: "source-resource-cleanup",
        message: "Source resource cleanup failed",
      }),
  }).pipe(Effect.catch((error) => Effect.logWarning(error.message, { code: error.code })))
  yield* cleanup.pipe(Effect.repeat(Schedule.spaced("5 minutes")), Effect.forkScoped)

  return {
    issue: (sourceId: string, asset: AssetReference, mediaType: string) =>
      Effect.tryPromise({
        try: async () => {
          // Bucket issuance by minute so repeated queries reuse one record instead of growing storage.
          const expiresAt = Math.floor(Date.now() / 60_000) * 60_000 + 600_000
          const text = JSON.stringify({ sourceId, asset, mediaType, expiresAt })
          const token = `${expiresAt}-${createHmac("sha256", key).update(text).digest("hex")}`
          const destination = join(root, token)
          if (!(await Bun.file(destination).exists())) {
            const iv = randomBytes(12)
            const cipher = createCipheriv("aes-256-gcm", key, iv)
            cipher.setAAD(Buffer.from(token))
            const body = Buffer.concat([cipher.update(text), cipher.final()])
            const temporary = `${destination}.${crypto.randomUUID()}.tmp`
            try {
              await writeFile(temporary, Buffer.concat([iv, cipher.getAuthTag(), body]), {
                mode: 0o600,
              })
              await rename(temporary, destination)
            } finally {
              await unlink(temporary).catch(() => {})
            }
          }
          return `/api/v1/source-assets?token=${token}`
        },
        catch: () =>
          new StorageError({
            code: "source-resource-write",
            message: "Could not prepare source page access",
          }),
      }),
    resolve: (token: string) =>
      Effect.tryPromise({
        try: async () => {
          const match = tokenPattern.exec(token)
          if (!match || Number(match[1]) <= Date.now()) throw new Error("Expired resource")
          const bytes = await readFile(join(root, token))
          const cipher = createDecipheriv("aes-256-gcm", key, bytes.subarray(0, 12))
          cipher.setAAD(Buffer.from(token))
          cipher.setAuthTag(bytes.subarray(12, 28))
          const resource = Schema.decodeUnknownSync(Resource)(
            JSON.parse(
              Buffer.concat([cipher.update(bytes.subarray(28)), cipher.final()]).toString(),
            ),
          )
          if (resource.expiresAt <= Date.now()) throw new Error("Expired resource")
          return resource
        },
        catch: () =>
          new RequestError({
            code: "source-asset-expired",
            message: "Reload the source page to obtain a fresh asset URL",
          }),
      }),
  }
})
