import { TelemetryClient } from "../client";
import type { Event } from "./types";

export class CreateTurboTelemetry extends TelemetryClient {
  trackOptionExample(value: string | undefined): Event | undefined {
    if (value) {
      return this.trackCliOption({
        option: "example",
        value,
      });
    }
  }

  trackOptionPackageManager(value: string | undefined): Event | undefined {
    if (value) {
      return this.trackCliOption({
        option: "package_manager",
        value,
      });
    }
  }

  trackOptionSkipInstall(value: boolean | undefined): Event | undefined {
    if (value) {
      return this.trackCliOption({
        option: "skip_install",
        value: value.toString(),
      });
    }
  }

  trackOptionSkipTransforms(value: boolean | undefined): Event | undefined {
    if (value) {
      return this.trackCliOption({
        option: "skip_transforms",
        value: value.toString(),
      });
    }
  }

  trackOptionTurboVersion(value: string | undefined): Event | undefined {
    if (value) {
      return this.trackCliOption({
        option: "turbo_version",
        value,
      });
    }
  }

  trackOptionExamplePath(value: string | undefined): Event | undefined {
    if (value) {
      return this.trackCliOption({
        option: "example_path",
        value,
      });
    }
  }

  // only track that the argument was provided, not what it was
  trackArgumentDirectory(provided: boolean): Event | undefined {
    if (provided) {
      return this.trackCliArgument({
        argument: "project_directory",
        value: "provided",
      });
    }
  }

  trackArgumentPackageManager(value: string | undefined): Event | undefined {
    if (value) {
      return this.trackCliArgument({
        argument: "package_manager",
        value,
      });
    }
  }
}
