import { Schema } from "effect"
import { HttpApi, HttpApiEndpoint, HttpApiGroup } from "effect/http-api"
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

export const MangaApi = HttpApi.make("MangaApi").add(
  HttpApiGroup.make("basic").add(
    HttpApiEndpoint.get("root", "/", {
      success: Schema.Struct({ message: Schema.String }),
    }),
    HttpApiEndpoint.get("health", "/health", {
      success: Schema.Struct({ status: Schema.Literal("ok") }),
    }),
  ),
  HttpApiGroup.make("catalog").add(
    HttpApiEndpoint.get("extensionCollections", "/api/v1/extension-collections", {
      success: Schema.Array(ExtensionCollection),
    }),
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
    }),
    HttpApiEndpoint.post("scan", "/api/v1/sources/:id/scan", {
      params: { id: Id },
      success: Job,
      error: [Missing, StorageError],
    }),
    HttpApiEndpoint.get("jobs", "/api/v1/jobs", {
      success: Schema.Array(Job),
      error: StorageError,
    }),
    HttpApiEndpoint.get("jobEvents", "/api/v1/jobs/:id/events", {
      params: { id: Id },
      success: Schema.Array(JobEvent),
      error: StorageError,
    }),
    HttpApiEndpoint.post("cancelJob", "/api/v1/jobs/:id/cancel", {
      params: { id: Id },
      success: Job,
      error: [Missing, StorageError],
    }),
    HttpApiEndpoint.post("retryJob", "/api/v1/jobs/:id/retry", {
      params: { id: Id },
      success: Job,
      error: [Missing, StorageError],
    }),
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
