import { Schema } from "effect"
import { HttpApi, HttpApiEndpoint, HttpApiGroup, OpenApi } from "effect/http-api"
import { SourceQuery } from "@chek/core"
import { PublicSourceResult } from "./source-models"
import { deliveryPaths } from "./delivery-docs"
import { ExtensionCollection, ExtensionManifest } from "@chek/extensions"
import {
  Id,
  Job,
  JobEvent,
  Missing,
  Publication,
  RequestError,
  Source,
  SourceInput,
  StorageError,
} from "../catalog/models"

export const MangaApi = HttpApi.make("MangaApi")
  .annotate(OpenApi.Transform, (spec) => ({ ...spec, paths: { ...spec.paths, ...deliveryPaths } }))
  .add(
    HttpApiGroup.make("basic").add(
      HttpApiEndpoint.get("root", "/", {
        success: Schema.Struct({ message: Schema.String }),
      }),
      HttpApiEndpoint.get("health", "/health", {
        success: Schema.Struct({ status: Schema.Literal("ok") }),
      }),
    ),
    HttpApiGroup.make("catalog").add(
      HttpApiEndpoint.post("sourceQuery", "/api/v1/sources/:id/query", {
        params: { id: Id },
        payload: SourceQuery,
        success: PublicSourceResult,
        error: [Missing, RequestError, StorageError],
      }).annotate(
        OpenApi.Description,
        "Browse/search a source, fetch series details, list chapter releases, or resolve ordered pages. Check the installed extension capabilities first. Keys and cursors are opaque and source-scoped. Results are cached for 30 seconds; concurrent identical requests share execution. Cover/page URLs expire after 10 minutes and require the same API authorization. Requery to refresh expired URLs. Upstream credentials and local paths never appear in this response.",
      ),
      HttpApiEndpoint.get("extensionCollections", "/api/v1/extension-collections", {
        success: Schema.Array(ExtensionCollection),
      }).annotate(
        OpenApi.Description,
        "Author collections are discovery metadata only. Listed packages are not installed automatically.",
      ),
      HttpApiEndpoint.get("extensions", "/api/v1/extensions", {
        success: Schema.Array(ExtensionManifest),
      }),
      HttpApiEndpoint.get("sources", "/api/v1/sources", {
        success: Schema.Array(Source),
        error: StorageError,
      }),
      HttpApiEndpoint.post("createSource", "/api/v1/sources", {
        payload: SourceInput,
        success: Source,
        error: [RequestError, StorageError],
      }).annotate(
        OpenApi.Description,
        "Validate extension configuration and explicit filesystem/HTTP grants, then persist a source. Secret configuration is excluded from responses. This does not start a scan or query.",
      ),
      HttpApiEndpoint.post("scan", "/api/v1/sources/:id/scan", {
        params: { id: Id },
        success: Job,
        error: [Missing, RequestError, StorageError],
      }).annotate(
        OpenApi.Description,
        "Queue a durable import for an extension with discover capability. Reuses an active job for the same source. Imports preserve originals and stable publication IDs; they do not parse files into reader pages.",
      ),
      HttpApiEndpoint.get("jobs", "/api/v1/jobs", {
        success: Schema.Array(Job),
        error: StorageError,
      }).annotate(
        OpenApi.Description,
        "Most recent 100 import jobs. A transient failure returns to queued with nextAttemptAt; automatic retries stop after three attempts. Counters include work across attempts. traceId identifies the latest execution trace.",
      ),
      HttpApiEndpoint.get("jobEvents", "/api/v1/jobs/:id/events", {
        params: { id: Id },
        success: Schema.Array(JobEvent),
        error: StorageError,
      }),
      HttpApiEndpoint.post("cancelJob", "/api/v1/jobs/:id/cancel", {
        params: { id: Id },
        success: Job,
        error: [Missing, StorageError],
      }).annotate(
        OpenApi.Description,
        "Cancel queued/running work. Running subprocesses observe cancellation on the next heartbeat (within five seconds). Already committed publications remain available.",
      ),
      HttpApiEndpoint.post("retryJob", "/api/v1/jobs/:id/retry", {
        params: { id: Id },
        success: Job,
        error: [Missing, StorageError],
      }).annotate(
        OpenApi.Description,
        "For failed/cancelled jobs, enqueue a new job using the last durable source checkpoint, or return the source's existing active job. Completed/running jobs are returned unchanged.",
      ),
      HttpApiEndpoint.get("publications", "/api/v1/publications", {
        query: {
          after: Schema.optional(Schema.String),
          search: Schema.optional(Schema.String.check(Schema.isMaxLength(256))),
          limit: Schema.optional(Schema.Int.check(Schema.isBetween({ minimum: 1, maximum: 100 }))),
        },
        success: Schema.Struct({
          items: Schema.Array(Publication),
          nextCursor: Schema.NullOr(Schema.String),
        }),
        error: StorageError,
      }),
      HttpApiEndpoint.get("publication", "/api/v1/publications/:id", {
        params: { id: Id },
        success: Publication,
        error: [Missing, StorageError],
      }),
    ),
  )
