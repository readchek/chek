import { Schema } from "effect"
import { Count, Sha256, Text } from "./primitives.js"

export const AssetReference = Schema.Union([
  Schema.Struct({ kind: Schema.Literal("file"), path: Text }),
  Schema.Struct({
    kind: Schema.Literal("http"),
    url: Text.check(Schema.isPattern(/^https?:\/\//)),
    headers: Schema.optional(
      Schema.Record(
        Schema.String.check(Schema.isPattern(/^(?:authorization|accept|x-[a-z0-9-]+)$/i)),
        Schema.String.check(Schema.isMaxLength(8192), Schema.isPattern(/^[^\r\n]*$/)),
      ).check(Schema.isMaxProperties(32)),
    ),
  }),
])
export type AssetReference = typeof AssetReference.Type

export const AssetGrants = Schema.Struct({
  readRoots: Schema.Array(Text).check(Schema.isMaxLength(16)),
  httpOrigins: Schema.Array(Text).check(Schema.isMaxLength(16)),
})
export type AssetGrants = typeof AssetGrants.Type

export const StoredAsset = Schema.Struct({ hash: Sha256, byteLength: Count })
export type StoredAsset = typeof StoredAsset.Type

export class AssetError extends Schema.TaggedError<AssetError>()("AssetError", {
  code: Text,
  message: Text,
}) {}
