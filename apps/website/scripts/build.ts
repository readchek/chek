import { rm } from "node:fs/promises"
import { resolve } from "node:path"
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
  external: ["*.ttf"],
})
if (!build.success) throw new AggregateError(build.logs, "Website build failed")
for (const name of ["bricolage-700.ttf", "dm-sans-400.ttf", "dm-sans-600.ttf"]) {
  await Bun.write(
    resolve(dist, "public/assets/fonts", name),
    Bun.file(resolve(root, "public/assets/fonts", name)),
  )
}

// Copy assets referenced by stable URLs (browser fallback and on-demand reader artwork).
await Bun.write(
  resolve(dist, "favicon.ico"),
  Bun.file(resolve(root, "public/assets/logos/favicon.ico")),
)
await Bun.write(
  resolve(dist, "assets/images/last-train.webp"),
  Bun.file(resolve(root, "public/assets/images/last-train.webp")),
)
await cleanBuildOutputDir(root)
await writeRootConfig(root, { accountId: config.accountId }, { isPreview: false, mode: undefined })
await writeWorkerConfig({ root, config: config.worker })
await writeAssets({ root, sourceDirectory: dist })
