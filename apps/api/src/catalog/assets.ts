import { AssetError, type AssetGrants, type PublicationInput } from "@chek/core"
import { constants } from "node:fs"
import { link, mkdir, open, realpath, stat, unlink, statfs } from "node:fs/promises"
import { dirname, join, resolve } from "node:path"
import type { SourceAsset } from "./models"
import { containsPath } from "../platform/paths"

function assetError(code: string, message: string) {
  return new AssetError({ code, message })
}

const reserveBytes = 256 * 1024 * 1024

export function assetPath(directory: string, hash: string) {
  return join(resolve(directory), "assets", hash.slice(0, 2), hash)
}

async function remoteStream(
  url: string,
  origins: readonly string[],
  signal: AbortSignal,
  headers: Readonly<Record<string, string>> = {},
): Promise<ReadableStream<Uint8Array>> {
  let target = new URL(url)
  const credentialOrigin = target.origin
  for (let redirects = 0; redirects <= 5; redirects++) {
    if (
      target.username ||
      target.password ||
      !origins.includes(target.origin) ||
      !["http:", "https:"].includes(target.protocol)
    ) {
      throw assetError("asset-grant", "Asset URL is outside the source's granted HTTP origins")
    }
    const response = await fetch(target, {
      redirect: "manual",
      signal,
      headers: {
        ...(target.origin === credentialOrigin ? headers : {}),
        "Accept-Encoding": "identity",
      },
    })
    if ([301, 302, 303, 307, 308].includes(response.status)) {
      await response.body?.cancel()
      const location = response.headers.get("location")
      if (!location) break
      target = new URL(location, target)
      continue
    }
    if (!response.ok || !response.body) {
      await response.body?.cancel()
      throw assetError("asset-http", `Source returned HTTP ${response.status}`)
    }
    return response.body
  }
  throw assetError(
    "asset-redirect",
    "Source exceeded redirect limit or omitted its redirect location",
  )
}

// Recognition is deliberately separate from parser/readiness support.
function verifySignature(prefix: Uint8Array, format: string) {
  const bytes = Buffer.from(prefix)
  const zipFormats = ["epub", "cbz", "fb2.zip", "docx", "odt"]
  if (
    zipFormats.includes(format) &&
    !bytes.subarray(0, 4).equals(Buffer.from([0x50, 0x4b, 0x03, 0x04]))
  )
    throw assetError("format-signature", "File does not have the expected ZIP signature")
  if (format === "pdf" && !bytes.subarray(0, 1024).includes(Buffer.from("%PDF-")))
    throw assetError("format-signature", "File does not have a PDF header")
  if (format === "cbr" && !bytes.subarray(0, 6).equals(Buffer.from("Rar!\x1a\x07", "binary")))
    throw assetError("format-signature", "File does not have a RAR header")
  if (
    format === "cb7" &&
    !bytes.subarray(0, 6).equals(Buffer.from([0x37, 0x7a, 0xbc, 0xaf, 0x27, 0x1c]))
  )
    throw assetError("format-signature", "File does not have a 7z header")
}

export async function storeAsset(
  publication: PublicationInput,
  source: AssetGrants,
  directory: string,
  maxBytes: number,
  signal: AbortSignal,
  previous: SourceAsset | null,
) {
  const temporaryDirectory = join(resolve(directory), "temporary")
  await mkdir(temporaryDirectory, { recursive: true, mode: 0o700 })
  const temporary = join(temporaryDirectory, crypto.randomUUID())
  let local: Awaited<ReturnType<typeof open>> | undefined
  let before: Awaited<ReturnType<typeof stat>> | undefined
  let output: Awaited<ReturnType<typeof open>> | undefined
  let reader: ReturnType<ReadableStream<Uint8Array>["getReader"]> | undefined
  let sourceFingerprint: string | null = null
  try {
    let stream: ReadableStream<Uint8Array>
    if (publication.asset.kind === "file") {
      const path = await realpath(publication.asset.path)
      if (!source.readRoots.some((root) => containsPath(root, path)))
        throw assetError("asset-grant", "File is outside the source's granted folders")
      local = await open(path, constants.O_RDONLY | constants.O_NOFOLLOW)
      before = await local.stat()
      if (!before.isFile() || before.size > maxBytes)
        throw assetError(
          "asset-size",
          "Source is not a regular file or exceeds the asset size limit",
        )
      sourceFingerprint = `${before.dev}:${before.ino}:${before.size}:${before.mtimeMs}:${before.ctimeMs}`
      if (previous?.sourceFingerprint === sourceFingerprint) {
        const cached = await stat(assetPath(directory, previous.assetHash)).catch(() => null)
        if (cached?.size === previous.byteLength) {
          const prefix = Buffer.alloc(1024)
          const { bytesRead } = await local.read(prefix, 0, prefix.length, 0)
          verifySignature(prefix.subarray(0, bytesRead), publication.format)
          return {
            hash: previous.assetHash,
            byteLength: previous.byteLength,
            sourceFingerprint,
            reused: true,
          }
        }
      }
      stream = Bun.file(local.fd).stream()
    } else {
      stream = await remoteStream(
        publication.asset.url,
        source.httpOrigins,
        signal,
        publication.asset.headers,
      )
    }
    const inputReader = stream.getReader()
    reader = inputReader
    output = await open(temporary, "wx", 0o600)
    const hasher = new Bun.CryptoHasher("sha256")
    let byteLength = 0
    let nextDiskCheck = 0
    let prefix = new Uint8Array(0)
    while (true) {
      signal.throwIfAborted()
      const chunk = await inputReader.read()
      if (chunk.done) break
      byteLength += chunk.value.byteLength
      if (byteLength > maxBytes)
        throw assetError("asset-size", "Asset exceeds the configured byte limit")
      if (prefix.byteLength < 1024)
        prefix = new Uint8Array(
          Buffer.concat([prefix, chunk.value.subarray(0, 1024 - prefix.byteLength)]),
        )
      if (byteLength >= nextDiskCheck) {
        const disk = await statfs(temporaryDirectory)
        if (disk.bavail * disk.bsize < reserveBytes + chunk.value.byteLength)
          throw assetError("storage-full", "Import paused because the disk reserve was reached")
        nextDiskCheck = byteLength + 16 * 1024 * 1024
      }
      hasher.update(chunk.value)
      let offset = 0
      while (offset < chunk.value.byteLength) {
        const { bytesWritten } = await output.write(
          chunk.value,
          offset,
          chunk.value.byteLength - offset,
        )
        if (bytesWritten === 0) throw assetError("asset-write", "Could not write imported bytes")
        offset += bytesWritten
      }
    }
    if (byteLength === 0) throw assetError("asset-empty", "Source file is empty")
    verifySignature(prefix, publication.format)
    if (local && before) {
      const after = await local.stat()
      if (
        before.size !== after.size ||
        before.mtimeMs !== after.mtimeMs ||
        before.ctimeMs !== after.ctimeMs ||
        byteLength !== after.size
      ) {
        throw assetError(
          "asset-changing",
          "Source changed during import; retry when the file is complete",
        )
      }
    }
    await output.sync()
    await output.close()
    output = undefined
    const hash = hasher.digest("hex")
    const destination = assetPath(directory, hash)
    const bucket = join(resolve(directory), "assets", hash.slice(0, 2))
    await mkdir(bucket, { recursive: true, mode: 0o700 })
    try {
      await link(temporary, destination)
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== "EEXIST") throw error
      if ((await stat(destination)).size !== byteLength)
        throw assetError("asset-integrity", "Existing asset has an unexpected size")
    }
    for (const parentPath of [bucket, dirname(bucket), resolve(directory)]) {
      const parent = await open(parentPath, "r")
      try {
        await parent.sync()
      } finally {
        await parent.close()
      }
    }
    return { hash, byteLength, sourceFingerprint, reused: false }
  } finally {
    await reader?.cancel().catch(() => {})
    await output?.close().catch(() => {})
    await local?.close().catch(() => {})
    await unlink(temporary).catch(() => {})
  }
}
