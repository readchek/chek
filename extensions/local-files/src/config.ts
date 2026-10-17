import { ExtensionContext, ExtensionError } from "@chek/extensions"
import { Effect, Schema } from "effect"
import { realpath, stat } from "node:fs/promises"
import { isAbsolute, relative, sep } from "node:path"

export const LocalConfig = Schema.Struct({ root: Schema.NonEmptyString })

export const resolveRoot = Effect.fn("local.resolveRoot")(function* (
  config: typeof LocalConfig.Type,
) {
  const { grants } = yield* ExtensionContext
  const root = yield* Effect.tryPromise({
    try: async () => {
      const path = await realpath(config.root)
      if (!(await stat(path)).isDirectory()) throw new Error("Not a directory")
      return path
    },
    catch: () =>
      new ExtensionError({
        code: "directory-unreadable",
        message: "Source folder cannot be opened",
        retryable: false,
      }),
  })

  const allowed = grants.readRoots.some((grant) => {
    const child = relative(grant, root)
    return child === "" || (!isAbsolute(child) && child !== ".." && !child.startsWith(`..${sep}`))
  })

  if (!allowed)
    return yield* new ExtensionError({
      code: "directory-grant",
      message: "Source folder needs a read grant",
      retryable: false,
    })

  return root
})
