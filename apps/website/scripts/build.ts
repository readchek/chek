import { rm } from "node:fs/promises"
import { relative, resolve } from "node:path"
import {
  cleanBuildOutputDir,
  writeAssets,
  writeRootConfig,
  writeWorkerConfig,
} from "@cloudflare/build-output-utils"
import config from "../cloudflare.config.ts"

const root = resolve(import.meta.dir, "..")
const dist = resolve(root, "dist")

await rm(dist, { recursive: true, force: true })
const build = await Bun.build({
  entrypoints: [resolve(root, "src/index.html")],
  outdir: dist,
  minify: true,
  external: ["*.woff2"],
})
if (!build.success) throw new AggregateError(build.logs, "Website build failed")

// Only versioned URLs receive immutable caching. HTML always checks for updates.
const immutablePaths = build.outputs
  .filter((output) => !output.path.endsWith(".html"))
  .map((output) => `/${relative(dist, output.path)}`)
for await (const name of new Bun.Glob("*.woff2").scan(resolve(root, "public/assets/fonts"))) {
  const path = `public/assets/fonts/${name}`
  await Bun.write(resolve(dist, path), Bun.file(resolve(root, path)))
  immutablePaths.push(`/${path}`)
}

// The full artwork stays off the initial download but uses a versioned URL too.
const artwork = Bun.file(resolve(root, "public/assets/images/last-train.webp"))
const hash = new Bun.CryptoHasher("sha256")
  .update(await artwork.arrayBuffer())
  .digest("hex")
  .slice(0, 12)
const artworkPath = `/assets/images/last-train-${hash}.webp`
await Bun.write(resolve(dist, artworkPath.slice(1)), artwork)
immutablePaths.push(artworkPath)
const index = Bun.file(resolve(dist, "index.html"))
await Bun.write(
  index,
  (await index.text()).replaceAll("/assets/images/last-train.webp", artworkPath),
)

// Conventional fallback favicon URL remains revalidated when the mark changes.
await Bun.write(
  resolve(dist, "favicon.ico"),
  Bun.file(resolve(root, "public/assets/logos/favicon.ico")),
)
await Bun.write(
  resolve(dist, "_headers"),
  immutablePaths
    .sort()
    .map((path) => `${path}\n  Cache-Control: public, max-age=31536000, immutable\n`)
    .join("\n"),
)

await cleanBuildOutputDir(root)
await writeRootConfig(root, { accountId: config.accountId }, { isPreview: false, mode: undefined })
await writeWorkerConfig({ root, config: config.worker })
await writeAssets({ root, sourceDirectory: dist })
