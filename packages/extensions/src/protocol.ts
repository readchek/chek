import { AssetGrants, PublicationInput, Text } from "@chek/core"
import { Schema } from "effect"
import { ExtensionConfig, ExtensionId, ExtensionVersion } from "./manifest.js"

export const ExtensionRequest = Schema.Struct({
  protocolVersion: Schema.Literal(1),
  operation: Schema.Literals(["validate", "discover"]),
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
})
export type ExtensionRequest = typeof ExtensionRequest.Type

export const DiscoveryEvent = Schema.Union([
  Schema.Struct({ type: Schema.Literal("publication"), publication: PublicationInput }),
  Schema.Struct({ type: Schema.Literal("warning"), code: Text, message: Text }),
])
export type DiscoveryEvent = typeof DiscoveryEvent.Type

export const ExtensionEvent = Schema.Union([
  DiscoveryEvent,
  Schema.Struct({ type: Schema.Literal("complete") }),
  Schema.Struct({
    type: Schema.Literal("failure"),
    code: Text,
    message: Text,
    retryable: Schema.Boolean,
  }),
])
export type ExtensionEvent = typeof ExtensionEvent.Type

export const protocolLimits = { requestBytes: 65536, eventBytes: 65536 } as const
