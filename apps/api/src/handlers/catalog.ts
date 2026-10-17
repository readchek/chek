import { Effect } from "effect"
import { HttpApiBuilder } from "effect/http-api"
import { MangaApi } from "../api/api"
import { CatalogStore } from "../catalog/store"
import { Missing, RequestError } from "../catalog/models"
import { SourceQueries } from "../extensions/queries"
import type { Series } from "@chek/core"
import { ExtensionRegistry } from "../extensions/registry"
import { ExtensionCollections } from "../extensions/collections"
import { ImportWakeup } from "../imports/wakeup"

export const CatalogHandlers = HttpApiBuilder.group(
  MangaApi,
  "catalog",
  Effect.fn(function* (handlers) {
    const store = yield* CatalogStore
    const registry = yield* ExtensionRegistry
    const collections = yield* ExtensionCollections
    const wakeup = yield* ImportWakeup
    const queries = yield* SourceQueries
    return handlers.handleAll({
      sourceQuery: Effect.fn("sources.publicQuery")(function* ({ params, payload }) {
        const result = yield* queries.query(params.id, payload)
        const series = ({ cover, ...value }: Series) => ({
          ...value,
          ...(cover ? { cover: queries.resourceUrl(params.id, cover, "image/*") } : {}),
        })
        switch (result.operation) {
          case "series":
            return { ...result, data: series(result.data) }
          case "browse":
          case "search":
            return { ...result, data: { ...result.data, items: result.data.items.map(series) } }
          case "pages":
            return {
              ...result,
              data: {
                ...result.data,
                items: result.data.items.map((page) => ({
                  ...page,
                  asset: queries.resourceUrl(params.id, page.asset, page.mediaType),
                })),
              },
            }
          case "chapters":
            return result
        }
      }),
      extensionCollections: () => Effect.succeed(collections.collections),
      extensions: () =>
        Effect.succeed([...registry.installed.values()].map((extension) => extension.manifest)),
      sources: () => store.call("sources", null),
      createSource: ({ payload }) =>
        registry
          .validateSource(payload)
          .pipe(Effect.flatMap((source) => store.call("createSource", source))),
      scan: Effect.fn("sources.scan")(function* ({ params }) {
        const source = yield* store.call("source", params.id)
        if (!source) return yield* new Missing({ message: "Source not found" })
        if (!registry.installed.get(source.extensionId)?.manifest.capabilities.includes("discover"))
          return yield* new RequestError({
            code: "capability-missing",
            message: "Source does not support file discovery",
          })
        const job = yield* store.call("enqueue", source.id)
        yield* wakeup.notify
        yield* Effect.logInfo("Import queued", {
          sourceId: source.id,
          jobId: job.id,
          extensionId: source.extensionId,
        })
        return job
      }),
      jobs: () => store.call("jobs", null),
      jobEvents: ({ params }) => store.call("events", params.id),
      cancelJob: Effect.fn("jobs.cancel")(function* ({ params }) {
        const job = yield* store.call("cancel", params.id)
        if (!job) return yield* new Missing({ message: "Job not found" })
        yield* Effect.logInfo("Import cancellation requested", { jobId: job.id })
        return job
      }),
      retryJob: Effect.fn("jobs.retry")(function* ({ params }) {
        const job = yield* store.call("retry", params.id)
        if (!job) return yield* new Missing({ message: "Job not found" })
        yield* wakeup.notify
        return job
      }),
      publications: Effect.fn("catalog.list")(function* ({ query }) {
        const limit = query.limit ?? 30
        const rows = yield* store.call("publications", {
          after: query.after ?? "",
          search: query.search ?? "",
          limit: limit + 1,
        })
        const items = rows.slice(0, limit)
        return { items, nextCursor: rows.length > limit ? items.at(-1)!.id : null }
      }),
      publication: Effect.fn("catalog.get")(function* ({ params }) {
        const publication = yield* store.call("publication", params.id)
        if (!publication) return yield* new Missing({ message: "Publication not found" })
        return publication
      }),
    })
  }),
)
