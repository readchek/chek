import { Text } from "@chek/core"
import { Schema } from "effect"

/** Expected source failure. Messages must be safe to show without credentials or response bodies. */
export class ExtensionError extends Schema.TaggedError<ExtensionError>()("ExtensionError", {
  code: Text,
  message: Text,
  retryable: Schema.Boolean,
}) {}

export const unsupported = (operation: string) =>
  new ExtensionError({
    code: "capability-missing",
    message: `Source does not support ${operation}`,
    retryable: false,
  })
