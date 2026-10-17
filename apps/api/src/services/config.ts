import { Config, Context, Layer, Schema } from "effect"

const config = Config.all({
  dataDirectory: Config.String("CHEK_DATA_DIR").pipe(Config.withDefault(".chek")),
  extensionsDirectory: Config.String("CHEK_EXTENSIONS_DIR").pipe(Config.withDefault("")),
  extensionPackages: Config.String("CHEK_EXTENSION_PACKAGES").pipe(Config.withDefault("")),
  collectionsDirectory: Config.String("CHEK_EXTENSION_COLLECTIONS_DIR").pipe(
    Config.withDefault(""),
  ),
  apiToken: Config.String("CHEK_API_TOKEN").pipe(Config.withDefault("")),
  maxAssetBytes: Config.schema(
    Schema.Int.check(Schema.isBetween({ minimum: 1, maximum: 100_000_000_000 })),
    "CHEK_MAX_ASSET_BYTES",
  ).pipe(Config.withDefault(2_000_000_000)),
  extensionTimeoutMs: Config.schema(
    Schema.Int.check(Schema.isBetween({ minimum: 1000, maximum: 86_400_000 })),
    "CHEK_EXTENSION_TIMEOUT_MS",
  ).pipe(Config.withDefault(3_600_000)),
  importConcurrency: Config.schema(
    Schema.Int.check(Schema.isBetween({ minimum: 1, maximum: 16 })),
    "CHEK_IMPORT_CONCURRENCY",
  ).pipe(Config.withDefault(2)),
  assetTimeoutMs: Config.schema(
    Schema.Int.check(Schema.isBetween({ minimum: 1000, maximum: 86_400_000 })),
    "CHEK_ASSET_TIMEOUT_MS",
  ).pipe(Config.withDefault(120000)),
  host: Config.String("HOST").pipe(Config.withDefault("127.0.0.1")),
  port: Config.schema(
    Schema.Int.check(Schema.isBetween({ minimum: 1, maximum: 65535 })),
    "PORT",
  ).pipe(Config.withDefault(8080)),
  logLevel: Config.LogLevel("LOG_LEVEL").pipe(Config.withDefault("Info")),
  logFormat: Config.Literals(["pretty", "json"], "LOG_FORMAT").pipe(Config.withDefault("pretty")),
  serviceName: Config.String("OTEL_SERVICE_NAME").pipe(Config.withDefault("chek-api")),
  otlpProtocol: Config.Literals(["http/protobuf", "http/json"], "OTEL_EXPORTER_OTLP_PROTOCOL").pipe(
    Config.withDefault("http/protobuf"),
  ),
})

export class ApiConfig extends Context.Service<ApiConfig, Config.Success<typeof config>>()(
  "@chek/api/ApiConfig",
) {
  static readonly layer = Layer.effect(ApiConfig, config)
}
