import { Schema } from "effect"

export const Text = Schema.NonEmptyString.check(Schema.isMaxLength(2048))
export const Identifier = Schema.NonEmptyString.check(Schema.isMaxLength(256))
export const Count = Schema.Int.check(Schema.isGreaterThanOrEqualTo(0))
export const Sha256 = Schema.String.check(Schema.isPattern(/^[a-f0-9]{64}$/))

// New formats do not require changes to the core's publication shape.
export const Format = Schema.String.check(
  Schema.isPattern(/^[a-z0-9]+(?:[.+-][a-z0-9]+)*$/),
  Schema.isMaxLength(64),
)
export const MediaType = Schema.String.check(
  Schema.isPattern(/^[a-zA-Z0-9!#$&^_.+-]+\/[a-zA-Z0-9!#$&^_.+-]+$/),
  Schema.isMaxLength(256),
)
