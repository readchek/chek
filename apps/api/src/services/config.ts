import { Config, Context, Layer, Schema } from "effect"

const config = Config.all({
  host: Config.String("HOST").pipe(Config.withDefault("127.0.0.1")),
  port: Config.schema(
    Schema.Int.check(Schema.isBetween({ minimum: 1, maximum: 65535 })),
    "PORT",
  ).pipe(Config.withDefault(8080)),
  logLevel: Config.LogLevel("LOG_LEVEL").pipe(Config.withDefault("Info")),
  logFormat: Config.Literals(["pretty", "json"], "LOG_FORMAT").pipe(Config.withDefault("pretty")),
  serviceName: Config.String("OTEL_SERVICE_NAME").pipe(Config.withDefault("chibi-manga-api")),
  otlpProtocol: Config.Literals(["http/protobuf", "http/json"], "OTEL_EXPORTER_OTLP_PROTOCOL").pipe(
    Config.withDefault("http/protobuf"),
  ),
})

export class ApiConfig extends Context.Service<ApiConfig, Config.Success<typeof config>>()(
  "@manga/api/ApiConfig",
) {
  static readonly layer = Layer.effect(ApiConfig, config)
}
