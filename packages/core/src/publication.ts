import { Schema } from "effect"
import { AssetReference } from "./asset.js"
import { Count, Format, Identifier, MediaType, Sha256, Text } from "./primitives.js"

export const Contributor = Schema.Struct({
  name: Text,
  roles: Schema.Array(Text).check(Schema.isMaxLength(16)),
})

export const PublicationMetadata = Schema.Struct({
  identifier: Schema.optional(Text),
  description: Schema.optional(Schema.String.check(Schema.isMaxLength(32768))),
  publisher: Schema.optional(Text),
  contributors: Schema.optional(Schema.Array(Contributor).check(Schema.isMaxLength(100))),
  subjects: Schema.optional(Schema.Array(Text).check(Schema.isMaxLength(100))),
  series: Schema.optional(
    Schema.Struct({
      name: Text,
      position: Schema.optional(Schema.Number.check(Schema.isGreaterThanOrEqualTo(0))),
    }),
  ),
  layout: Schema.optional(Schema.Literals(["fixed", "reflowable"])),
  readingProgression: Schema.optional(Schema.Literals(["ltr", "rtl", "auto"])),
})
export type PublicationMetadata = typeof PublicationMetadata.Type

/** Input accepted by ingestion, regardless of which adapter supplied it. */
export const PublicationInput = Schema.Struct({
  sourceKey: Text,
  title: Text,
  format: Format,
  mediaType: MediaType,
  authors: Schema.Array(Text).check(Schema.isMaxLength(100)),
  language: Schema.optional(Text),
  metadata: Schema.optional(PublicationMetadata),
  asset: AssetReference,
})
export type PublicationInput = typeof PublicationInput.Type

/** Catalog identity and revision are assigned by the core, never by an extension. */
export const Publication = Schema.Struct({
  id: Identifier,
  sourceId: Identifier,
  sourceKey: Text,
  title: Text,
  format: Format,
  mediaType: MediaType,
  authors: Schema.Array(Text).check(Schema.isMaxLength(100)),
  language: Schema.NullOr(Text),
  metadata: PublicationMetadata,
  assetHash: Sha256,
  byteLength: Count,
  status: Schema.Literal("stored"),
  createdAt: Count,
  updatedAt: Count,
})
export type Publication = typeof Publication.Type
