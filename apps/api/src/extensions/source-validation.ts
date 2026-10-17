import type { ExtensionManifest } from "@chek/extensions"
import { Effect } from "effect"
import { realpath, stat } from "node:fs/promises"
import { RequestError, type SourceInput } from "../catalog/models"

export const validateFields = Effect.fnUntraced(function* (
  source: SourceInput,
  manifest: ExtensionManifest,
) {
  const fields = manifest.configFields

  for (const field of fields) {
    if (field.required && !source.config[field.key]?.trim()) {
      return yield* new RequestError({
        code: "configuration-required",
        message: `${field.label} is required`,
      })
    }
  }

  if (Object.keys(source.config).some((key) => !fields.some((field) => field.key === key))) {
    return yield* new RequestError({
      code: "configuration-unknown",
      message: "Unknown extension configuration field",
    })
  }
})

export const normalizeGrants = (source: SourceInput) =>
  Effect.tryPromise({
    try: async (): Promise<SourceInput> => {
      const readRoots = await Promise.all(
        source.readRoots.map(async (root) => {
          const path = await realpath(root)
          if (!(await stat(path)).isDirectory()) throw new Error("Read grant must name a directory")
          return path
        }),
      )

      const httpOrigins = source.httpOrigins.map((origin) => {
        const url = new URL(origin)
        if (
          !["https:", "http:"].includes(url.protocol) ||
          url.username ||
          url.password ||
          url.search ||
          url.hash ||
          url.pathname !== "/"
        ) {
          throw new Error("Grant must be an HTTP origin")
        }
        return url.origin
      })

      return {
        ...source,
        readRoots: [...new Set(readRoots)],
        httpOrigins: [...new Set(httpOrigins)],
      }
    },
    catch: () =>
      new RequestError({
        code: "source-grants",
        message: "Check folder paths, folder read grants, and HTTP origin grants",
      }),
  })
