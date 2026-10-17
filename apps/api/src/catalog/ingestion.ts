import { AssetError, PublicationInput, type AssetGrants } from "@chek/core"
import { Context, Effect, Layer, Metric, Schema } from "effect"
import { ApiConfig } from "../services/config"
import { storeAsset } from "./assets"
import { CatalogStore } from "./store"
import { RequestError, type Job, type StorageError } from "./models"

const importedBytes = Metric.counter("chek.import.bytes")
const importedFiles = Metric.counter("chek.import.files")

export interface ImportContext {
  readonly job: Job
  readonly grants: AssetGrants
  readonly signal: AbortSignal
}

/** Ingests core records without knowing where they were discovered. */
export class PublicationIngestion extends Context.Service<
  PublicationIngestion,
  {
    readonly ingest: (
      input: PublicationInput,
      context: ImportContext,
    ) => Effect.Effect<void, RequestError | StorageError>
  }
>()("@chek/api/PublicationIngestion") {
  static readonly layer = Layer.effect(
    PublicationIngestion,
    Effect.gen(function* () {
      const store = yield* CatalogStore
      const config = yield* ApiConfig
      const decode = Schema.decodeUnknownEffect(PublicationInput)

      const ingest = Effect.fn("imports.asset")(function* (
        input: PublicationInput,
        context: ImportContext,
      ) {
        const publication = yield* decode(input, { onExcessProperty: "error" }).pipe(
          Effect.mapError(
            () =>
              new RequestError({
                code: "publication-invalid",
                message: "Invalid publication data",
              }),
          ),
        )
        const { job, grants, signal } = context

        yield* Effect.annotateCurrentSpan({
          "chek.job.id": job.id,
          "chek.format": publication.format,
          "chek.source_key_hash": new Bun.CryptoHasher("sha256")
            .update(publication.sourceKey)
            .digest("hex"),
        })

        const previous = yield* store.call("sourceAsset", {
          sourceId: job.sourceId,
          sourceKey: publication.sourceKey,
        })

        const asset = yield* Effect.tryPromise({
          try: (interrupted) =>
            storeAsset(
              publication,
              grants,
              config.dataDirectory,
              config.maxAssetBytes,
              AbortSignal.any([signal, interrupted, AbortSignal.timeout(config.assetTimeoutMs)]),
              previous,
            ),
          catch: (error) =>
            new RequestError({
              code: error instanceof AssetError ? error.code : "asset-import",
              message:
                error instanceof AssetError
                  ? error.message
                  : "Asset import failed; check access, availability, and disk space",
            }),
        })

        const result = yield* store.call("import", {
          jobId: job.id,
          attempt: job.attempts,
          publication: {
            sourceId: job.sourceId,
            sourceKey: publication.sourceKey,
            title: publication.title,
            format: publication.format,
            mediaType: publication.mediaType,
            authors: [...publication.authors],
            language: publication.language ?? null,
            metadata: publication.metadata ?? {},
            assetHash: asset.hash,
            byteLength: asset.byteLength,
            sourceFingerprint: asset.sourceFingerprint,
          },
        })

        if (!asset.reused) yield* Metric.update(importedBytes, asset.byteLength)
        yield* Metric.update(importedFiles, 1)
        yield* Effect.logDebug("Asset imported", {
          jobId: job.id,
          assetHash: asset.hash,
          bytes: asset.byteLength,
          result,
        })
      })

      return PublicationIngestion.of({ ingest })
    }),
  )
}
