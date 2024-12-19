import { type CreateTurboTelemetry } from "./create-turbo";
import { type TurboIgnoreTelemetry } from "./turbo-ignore";

export interface TelemetryClientClasses {
  "create-turbo": typeof CreateTurboTelemetry;
  "turbo-ignore": typeof TurboIgnoreTelemetry;
}

export interface PackageInfo {
  name: keyof TelemetryClientClasses;
  version: string;
}

export interface Event {
  id: string;
  key: string;
  value: string;
  package_name: string;
  package_version: string;
  parent_id: string | undefined;
}
