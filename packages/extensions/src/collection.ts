import { Text } from "@chek/core"
import { Schema } from "effect"
import { ExtensionAuthor, ExtensionId, ExtensionPackageName, ExtensionVersion } from "./manifest.js"

/** Distribution metadata only. A collection never grants permission to run code. */
export const ExtensionRelease = Schema.Struct({
  id: ExtensionId,
  version: ExtensionVersion,
  package: ExtensionPackageName,
  integrity: Schema.String.check(Schema.isPattern(/^sha512-[A-Za-z0-9+/]{86}==$/)),
})
export type ExtensionRelease = typeof ExtensionRelease.Type

export const ExtensionCollection = Schema.Struct({
  schemaVersion: Schema.Literal(1),
  id: ExtensionId,
  name: Text,
  author: ExtensionAuthor,
  extensions: Schema.Array(ExtensionRelease).check(Schema.isMaxLength(1000)),
})
export type ExtensionCollection = typeof ExtensionCollection.Type
