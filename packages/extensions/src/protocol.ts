import { AssetGrants, PublicationInput, SourceQuery, SourceResult, Text } from "@chek/core"
import { Schema } from "effect"
import { ExtensionConfig, ExtensionId, ExtensionVersion } from "./manifest.js"

const RequestFields = {
  protocolVersion: Schema.Literal(1),
  sourceId: Schema.optional(Text),
  extensionId: ExtensionId,
  extensionVersion: ExtensionVersion,
  config: ExtensionConfig,
  grants: AssetGrants,
  trace: Schema.optional(
    Schema.Struct({
      traceId: Schema.String.check(Schema.isPattern(/^[a-f0-9]{32}$/)),
      spanId: Schema.String.check(Schema.isPattern(/^[a-f0-9]{16}$/)),
    }),
  ),
}
export const ExtensionRequest = Schema.Union([
  Schema.Struct({ ...RequestFields, operation: Schema.Literals(["validate", "discover"]) }),
  Schema.Struct({ ...RequestFields, operation: Schema.Literal("query"), query: SourceQuery }),
])
export type ExtensionRequest = typeof ExtensionRequest.Type

export const DiscoveryEvent = Schema.Union([
  Schema.Struct({ type: Schema.Literal("publication"), publication: PublicationInput }),
  Schema.Struct({ type: Schema.Literal("warning"), code: Text, message: Text }),
])
export type DiscoveryEvent = typeof DiscoveryEvent.Type

export const ExtensionEvent = Schema.Union([
  DiscoveryEvent,
  Schema.Struct({ type: Schema.Literal("result"), result: SourceResult }),
  Schema.Struct({ type: Schema.Literal("complete") }),
  Schema.Struct({
    type: Schema.Literal("failure"),
    code: Text,
    message: Text,
    retryable: Schema.Boolean,
  }),
])
export type ExtensionEvent = typeof ExtensionEvent.Type

export const protocolLimits = { requestBytes: 65536, eventBytes: 4 * 1024 * 1024 } as const
