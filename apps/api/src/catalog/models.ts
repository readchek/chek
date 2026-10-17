import { ExtensionConfig, ExtensionId } from "@chek/extensions"
import { AssetGrants, Count, Identifier, Publication } from "@chek/core"
export { Publication } from "@chek/core"
import { Schema } from "effect"

export const Id = Identifier
export const SourceInput = Schema.Struct({
  name: Schema.NonEmptyString.check(Schema.isMaxLength(256)),
  extensionId: ExtensionId,
  config: ExtensionConfig,
  ...AssetGrants.fields,
})
export type SourceInput = typeof SourceInput.Type
export const Source = Schema.Struct({
  id: Id,
  name: Schema.String,
  extensionId: Id,
  createdAt: Schema.Number,
})
export type Source = typeof Source.Type
export const StoredSource = Schema.Struct({ ...Source.fields, ...SourceInput.fields })
export type StoredSource = typeof StoredSource.Type

export const Job = Schema.Struct({
  id: Id,
  sourceId: Id,
  state: Schema.Literals(["queued", "running", "completed", "failed", "cancelled"]),
  attempts: Count,
  imported: Count,
  skipped: Count,
  warnings: Count,
  createdAt: Count,
  updatedAt: Count,
  errorCode: Schema.NullOr(Schema.String),
  errorMessage: Schema.NullOr(Schema.String),
  traceId: Schema.NullOr(Schema.String),
})
export type Job = typeof Job.Type

export type ImportedPublication = Omit<Publication, "id" | "createdAt" | "updatedAt" | "status"> & {
  sourceFingerprint: string | null
}
export interface SourceAsset {
  assetHash: string
  byteLength: number
  sourceFingerprint: string | null
}

export const JobEvent = Schema.Struct({
  id: Schema.Number,
  jobId: Id,
  code: Schema.String,
  message: Schema.String,
  createdAt: Schema.Number,
})
export type JobEvent = typeof JobEvent.Type

export class RequestError extends Schema.TaggedError<RequestError>()(
  "RequestError",
  {
    code: Schema.String,
    message: Schema.String,
  },
  { httpApiStatus: 400 },
) {}

export class Missing extends Schema.TaggedError<Missing>()(
  "Missing",
  {
    message: Schema.String,
  },
  { httpApiStatus: 404 },
) {}

export class StorageError extends Schema.TaggedError<StorageError>()(
  "StorageError",
  {
    message: Schema.String,
    code: Schema.String,
  },
  { httpApiStatus: 503 },
) {}

export interface StoreOperations {
  sources: { input: null; output: Source[] }
  source: { input: string; output: StoredSource | null }
  createSource: { input: SourceInput; output: Source }
  enqueue: { input: string; output: Job }
  jobs: { input: null; output: Job[] }
  job: { input: string; output: Job | null }
  events: { input: string; output: JobEvent[] }
  cancel: { input: string; output: Job | null }
  retry: { input: string; output: Job | null }
  claim: { input: null; output: Job | null }
  heartbeat: { input: { id: string; attempt: number }; output: boolean }
  trace: { input: { id: string; traceId: string; attempt: number }; output: null }
  finish: {
    input: { id: string; attempt: number; errorCode: string | null; errorMessage: string | null }
    output: null
  }
  event: { input: { id: string; attempt: number; code: string; message: string }; output: null }
  import: {
    input: { jobId: string; attempt: number; publication: ImportedPublication }
    output: "imported" | "skipped"
  }
  publications: { input: { after: string; search: string; limit: number }; output: Publication[] }
  publication: { input: string; output: Publication | null }
  asset: { input: string; output: { hash: string; byteLength: number } | null }
  sourceAsset: { input: { sourceId: string; sourceKey: string }; output: SourceAsset | null }
}
