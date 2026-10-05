import { BunRuntime } from "@effect/platform-bun"
import { Layer } from "effect"
import { AppLive } from "./app"

Layer.launch(AppLive).pipe(BunRuntime.runMain)
