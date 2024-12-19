import { name } from "ci-info";
import { TelemetryClient } from "../client";
import type { Event } from "./types";

const TASK_ALLOWLIST: Readonly<Array<string>> = [
  "build",
  "test",
  "lint",
  "typecheck",
  "checktypes",
  "check-types",
  "type-check",
  "check",
] as const;

export class TurboIgnoreTelemetry extends TelemetryClient {
  trackCI(): Event | undefined {
    return this.track({
      key: "ci",
      value: name ?? "unknown",
    });
  }

  /**
   * Track the workspace argument if it's provided.
   * We only track if it's provided, not what it was
   */
  trackArgumentWorkspace(provided: boolean): Event | undefined {
    if (provided) {
      return this.trackCliArgument({
        argument: "workspace",
        value: "provided",
      });
    }
  }

  /**
   * Track the task option if it's provided.
   * We only track the exact task name if it's in the allowlist
   * Otherwise, we track it as "other"
   */
  trackOptionTask(value: string | undefined): Event | undefined {
    if (value) {
      return this.trackCliOption({
        option: "task",
        value: TASK_ALLOWLIST.includes(value) ? value : "other",
      });
    }
  }

  trackOptionFallback(value: string | undefined): Event | undefined {
    if (value) {
      return this.trackCliOption({
        option: "fallback",
        value,
      });
    }
  }

  /**
   * Track the directory argument if it's provided.
   * We only track if it's provided, not what it was
   */
  trackOptionDirectory(value: string | undefined): Event | undefined {
    if (value) {
      return this.trackCliOption({
        option: "directory",
        value: "custom",
      });
    }
  }

  trackOptionMaxBuffer(value: number | undefined): Event | undefined {
    if (value !== undefined) {
      return this.trackCliOption({
        option: "max_buffer",
        value: value.toString(),
      });
    }
  }
}
