import { Layer } from "effect"
import { TelemetryLive } from "./observability/telemetry"
import { ApiServerLive } from "./server"
import { ApiConfig } from "./services/config"
import { CatalogStore } from "./catalog/store"
import { ExtensionRegistry } from "./extensions/registry"
import { ImportWorkerLive } from "./imports/worker"
import { PublicationIngestion } from "./catalog/ingestion"
import { ExtensionHost } from "./extensions/host"
import { ExtensionCollections } from "./extensions/collections"
import { ImportWakeup } from "./imports/wakeup"
import { SourceQueries } from "./extensions/queries"

export const AppLive = Layer.merge(ApiServerLive, ImportWorkerLive).pipe(
  Layer.provide(SourceQueries.layer),
  Layer.provide([PublicationIngestion.layer, ExtensionRegistry.layer, ExtensionCollections.layer]),
  Layer.provide([CatalogStore.layer, ExtensionHost.layer, ImportWakeup.layer]),
  Layer.provide(TelemetryLive),
  Layer.provide(ApiConfig.layer),
)
