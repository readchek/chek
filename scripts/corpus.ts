import { lstat } from "node:fs/promises"
import { dirname, join, resolve } from "node:path"
import { BunRuntime, BunServices } from "@effect/platform-bun"
import { Console, Effect, FileSystem, Schedule, Schema, Stream } from "effect"
import type { Cause, PlatformError } from "effect"
import { Command, Flag } from "effect/cli"
import { ChildProcess, ChildProcessSpawner } from "effect/process"

const root = resolve(import.meta.dir, "..")
const cache = join(root, ".chek/corpus")
const safePath = Schema.String.check(
  Schema.isPattern(/^(?!\/)(?!.*\\)(?!.*[\r\n])(?!.*(?:^|\/)\.{1,2}(?:\/|$))(?!.*\/\/).+[^/]$/),
)
const checksum = Schema.String.check(Schema.isPattern(/^[a-f0-9]{64}$/))
const file = Schema.Struct({
  bytes: Schema.Int.check(Schema.isGreaterThanOrEqualTo(0)),
  sha256: checksum,
})
const Lock = Schema.fromJsonString(
  Schema.Struct({
    version: Schema.Literal(1),
    bucket: Schema.NonEmptyString,
    archive: Schema.Struct({
      ...file.fields,
      parts: Schema.Array(Schema.Struct({ ...file.fields, key: safePath })),
    }),
    files: Schema.Array(Schema.Struct({ ...file.fields, path: safePath })),
  }),
)
class CorpusError extends Schema.TaggedError<CorpusError>()("CorpusError", {
  message: Schema.String,
}) {}
const fail = (message: string) => new CorpusError({ message })

const digest = Effect.fn("corpus.digest")(function* (path: string) {
  const fs = yield* FileSystem.FileSystem
  const hash = new Bun.CryptoHasher("sha256")
  yield* fs.stream(path).pipe(
    Stream.runForEach((chunk) =>
      Effect.sync(() => {
        hash.update(chunk)
      }),
    ),
  )
  return hash.digest("hex")
})

const regular = Effect.fn("corpus.regular")(function* (path: string) {
  const info = yield* Effect.tryPromise(() => lstat(path))
  return info.isFile()
})

const matches = Effect.fn("corpus.matches")(function* (path: string, expected: typeof file.Type) {
  const fs = yield* FileSystem.FileSystem
  if (!(yield* fs.exists(path)) || !(yield* regular(path))) return false
  return (
    Number((yield* fs.stat(path)).size) === expected.bytes &&
    (yield* digest(path)) === expected.sha256
  )
})

const verify = Effect.fn("corpus.verify")(function* (
  directory: string,
  files: typeof Lock.Type.files,
) {
  const fs = yield* FileSystem.FileSystem
  const expected = new Map(files.map((entry) => [entry.path, entry]))
  const info = yield* Effect.tryPromise(() => lstat(directory))
  if (!info.isDirectory()) return yield* fail("Corpus destination must be a directory")
  let count = 0
  const walk = Effect.fnUntraced(function* (
    relative: string,
  ): Effect.fn.Return<
    void,
    CorpusError | Cause.UnknownError | PlatformError.PlatformError,
    FileSystem.FileSystem
  > {
    for (const name of yield* fs.readDirectory(join(directory, relative))) {
      const path = relative ? `${relative}/${name}` : name
      const info = yield* Effect.tryPromise(() => lstat(join(directory, path)))
      if (info.isDirectory()) yield* walk(path)
      else {
        const entry = expected.get(path)
        if (!entry || !info.isFile() || !(yield* matches(join(directory, path), entry))) {
          return yield* fail(
            `Changed or unexpected corpus file: ${path}. Move the corpus aside before restoring.`,
          )
        }
        count++
      }
    }
  })
  yield* walk("")
  if (count !== files.length)
    return yield* fail("Corpus files are missing. Move the corpus aside before restoring.")
  yield* Console.log(`Verified ${count} corpus files at ${directory}`)
})

const capture = Effect.fn("corpus.capture")(function* (command: string, args: string[]) {
  const spawner = yield* ChildProcessSpawner.ChildProcessSpawner
  const child = yield* spawner.spawn(ChildProcess.make(command, args, { stderr: "inherit" }))
  const output = yield* child.stdout.pipe(Stream.decodeText(), Stream.mkString)
  if ((yield* child.exitCode) !== 0) return yield* fail(`${command} failed`)
  return output.trimEnd().split("\n")
}, Effect.scoped)

const setup = Effect.fn("corpus.setup")(function* (destination: string, verifyOnly: boolean) {
  const fs = yield* FileSystem.FileSystem
  const spawner = yield* ChildProcessSpawner.ChildProcessSpawner
  const lock = yield* Schema.decodeUnknownEffect(Lock)(
    yield* fs.readFileString(join(root, "test-data/corpus.lock.json")),
  )
  const names = new Set(lock.files.map((entry) => entry.path))
  if (names.size !== lock.files.length || !names.size || !lock.archive.parts.length)
    return yield* fail("Invalid corpus inventory")
  if (yield* fs.exists(destination)) return yield* verify(destination, lock.files)
  if (verifyOnly) return yield* fail(`Corpus missing at ${destination}. Run bun run corpus:setup.`)
  yield* fs.makeDirectory(cache, { recursive: true })
  const mutex = join(cache, "setup.lock")
  yield* Effect.acquireRelease(
    fs
      .makeDirectory(mutex)
      .pipe(
        Effect.mapError(() =>
          fail(`Another setup may be running. If it stopped, remove ${mutex}.`),
        ),
      ),
    () => fs.remove(mutex, { recursive: true, force: true }).pipe(Effect.orDie),
  )
  const partPaths: string[] = []
  for (const part of lock.archive.parts) {
    const path = join(cache, part.sha256)
    partPaths.push(path)
    if (yield* matches(path, part)) continue
    const partial = `${path}.partial`
    yield* Effect.acquireRelease(Effect.void, () =>
      fs.remove(partial, { force: true }).pipe(Effect.orDie),
    )
    yield* Effect.gen(function* () {
      yield* Console.log(`Downloading ${part.key}`)
      const child = yield* spawner.spawn(
        ChildProcess.make(
          "cf",
          ["r2", "objects", "get", part.key, "--bucket-name", lock.bucket, "--quiet"],
          { stderr: "inherit" },
        ),
      )
      yield* child.stdout.pipe(Stream.run(fs.sink(partial)))
      if ((yield* child.exitCode) !== 0 || !(yield* matches(partial, part)))
        return yield* fail("Download failed or checksum mismatch. Check cf auth login and retry.")
      yield* fs.rename(partial, path)
    }).pipe(Effect.scoped, Effect.retry(Schedule.recurs(2)))
  }
  const archive = join(cache, `${lock.archive.sha256}.tar.gz`)
  if (!(yield* matches(archive, lock.archive))) {
    const partial = `${archive}.partial`
    yield* Effect.acquireRelease(Effect.void, () =>
      fs.remove(partial, { force: true }).pipe(Effect.orDie),
    )
    yield* Stream.fromIterable(partPaths).pipe(
      Stream.flatMap((path) => fs.stream(path)),
      Stream.run(fs.sink(partial)),
    )
    if (!(yield* matches(partial, lock.archive))) return yield* fail("Archive checksum mismatch")
    yield* fs.rename(partial, archive)
  }
  const listed = yield* capture("tar", ["-tzf", archive])
  const types = yield* capture("tar", ["-tvzf", archive])
  if (
    listed.length !== names.size ||
    new Set(listed).size !== names.size ||
    listed.some((name) => !names.has(name)) ||
    types.length !== listed.length ||
    types.some((line) => !line.startsWith("-"))
  ) {
    return yield* fail("Archive contains unexpected paths or non-regular entries")
  }
  yield* fs.makeDirectory(dirname(destination), { recursive: true })
  const staging = `${destination}.staging-${process.pid}`
  yield* Effect.acquireRelease(fs.makeDirectory(staging), () =>
    fs.remove(staging, { recursive: true, force: true }).pipe(Effect.orDie),
  )
  yield* capture("tar", [
    "-xzf",
    archive,
    "-C",
    staging,
    "--no-same-owner",
    "--no-same-permissions",
  ])
  yield* verify(staging, lock.files)
  if (yield* fs.exists(destination))
    return yield* fail("Destination appeared during setup; refusing to replace it")
  yield* fs.rename(staging, destination)
  yield* Console.log(`Corpus ready at ${destination}`)
}, Effect.scoped)

Command.make(
  "corpus",
  {
    verify: Flag.Boolean("verify").pipe(Flag.withDefault(false)),
    destination: Flag.String("destination").pipe(
      Flag.withDefault(join(root, "test-data/reader-corpus")),
    ),
  },
  ({ destination, verify }) => setup(resolve(destination), verify),
).pipe(
  Command.withDescription("Restore or verify the pinned private reader corpus"),
  Command.run({ version: "1.0.0" }),
  Effect.provide(BunServices.layer),
  BunRuntime.runMain,
)
