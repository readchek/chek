import { Text } from "@chek/core"
import { Schema } from "effect"

export const ExtensionId = Schema.String.check(
  Schema.isPattern(/^[a-z0-9]+(?:[.-][a-z0-9]+)*$/),
  Schema.isMaxLength(128),
)
export const ExtensionVersion = Schema.String.check(
  Schema.isPattern(/^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-[0-9A-Za-z.-]+)?$/),
  Schema.isMaxLength(128),
)

export const ConfigField = Schema.Struct({
  key: Text,
  label: Text,
  required: Schema.Boolean,
  secret: Schema.Boolean,
  grant: Schema.optional(Schema.Literals(["read-directory", "http-origin"])),
})

export const ExtensionAuthor = Schema.Struct({ name: Text, url: Schema.optional(Text) })

export const ExtensionPackageName = Schema.String.check(
  Schema.isPattern(/^(?:@[a-z0-9._-]+\/)?[a-z0-9][a-z0-9._-]*$/),
  Schema.isMaxLength(214),
)

export const ExtensionManifest = Schema.Struct({
  protocolVersion: Schema.Literal(1),
  id: ExtensionId,
  name: Text,
  version: ExtensionVersion,
  description: Text,
  entrypoint: Text,
  author: Schema.optional(ExtensionAuthor),
  capabilities: Schema.Array(Schema.Literal("discover")).check(
    Schema.isMinLength(1),
    Schema.isMaxLength(1),
  ),
  configFields: Schema.Array(ConfigField).check(Schema.isMaxLength(32)),
})
export type ExtensionManifest = typeof ExtensionManifest.Type

export const ExtensionConfig = Schema.Record(
  Schema.String,
  Schema.String.check(Schema.isMaxLength(8192)),
).check(Schema.isMaxProperties(32))
export type ExtensionConfig = typeof ExtensionConfig.Type
