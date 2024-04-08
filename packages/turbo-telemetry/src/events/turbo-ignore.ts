import { TelemetryClient } from "../client";
import type { Event } from "./types";

export class TurboIgnoreTelemetry extends TelemetryClient {
  trackExecutionEnv(): Event | undefined {
    return this.track({
      key: "execution_env",
      value: process.env.VERCEL === "1" ? "vercel" : "local",
    });
  }
}
