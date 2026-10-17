import { ExtensionManifest, ExtensionPackageName } from "@chek/extensions"
import { Effect, Schema } from "effect"
import { readdir, realpath, stat } from "node:fs/promises"
import { dirname, resolve } from "node:path"
import { fileURLToPath } from "node:url"
import { RequestError } from "../catalog/models"
import { containsPath } from "../platform/paths"

export interface InstalledExtension {
  readonly manifest: ExtensionManifest
  readonly entrypoint: string
}

interface InstallationConfig {
  readonly extensionsDirectory: string
  readonly extensionPackages: string
}

const decodeManifest = Schema.decodeUnknownSync(ExtensionManifest)
const decodePackage = Schema.decodeUnknownSync(ExtensionPackageName)

async function loadDirectory(directory: string): Promise<InstalledExtension> {
  const file = Bun.file(resolve(directory, "extension.json"))
  if (file.size > 65536) throw new Error("Extension manifest exceeds 64 KiB")

  const manifest = decodeManifest(await file.json(), { onExcessProperty: "error" })
  const entrypoint = await realpath(resolve(directory, manifest.entrypoint))
  if (!containsPath(directory, entrypoint) || !(await stat(entrypoint)).isFile()) {
    throw new Error("Extension entrypoint must be a file inside its package")
  }

  const keys = manifest.configFields.map((field) => field.key)
  if (new Set(keys).size !== keys.length) throw new Error("Duplicate configuration key")

  return { manifest, entrypoint }
}

export const loadInstalledExtensions = (config: InstallationConfig) =>
  Effect.tryPromise({
    try: async () => {
      const directories = [
        dirname(fileURLToPath(import.meta.resolve("@chek/extension-local-files/extension.json"))),
      ]
      const packages = config.extensionPackages
        .split(",")
        .map((value) => value.trim())
        .filter(Boolean)

      for (const input of packages) {
        const name = decodePackage(input)
        directories.push(dirname(fileURLToPath(import.meta.resolve(`${name}/extension.json`))))
      }

      if (config.extensionsDirectory) {
        const root = await realpath(config.extensionsDirectory)
        for (const entry of await readdir(root, { withFileTypes: true })) {
          if (entry.isDirectory()) directories.push(resolve(root, entry.name))
        }
      }

      const installed = new Map<string, InstalledExtension>()
      const seen = new Set<string>()

      for (const directory of directories) {
        const root = await realpath(directory)
        if (seen.has(root)) continue
        seen.add(root)

        const extension = await loadDirectory(root)
        if (installed.has(extension.manifest.id)) throw new Error("Duplicate extension ID")
        installed.set(extension.manifest.id, extension)
      }

      return installed
    },
    catch: () =>
      new RequestError({
        code: "extension-installation",
        message:
          "Cannot load installed extensions; check manifests, entrypoints, package exports, and duplicate IDs",
      }),
  })
