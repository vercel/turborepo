import { TelemetryConfig } from "./config";
import { TelemetryClient, type PackageInfo } from "./client";

const TELEMETRY_API = "https://telemetry.vercel.com";

export async function initTelemetry(packageInfo: PackageInfo) {
  // read the config
  const config = await TelemetryConfig.fromDefaultConfig();
  config.showAlert();
  // initialize the client
  const telemetry = new TelemetryClient(TELEMETRY_API, packageInfo, config);

  return { telemetry };
}
