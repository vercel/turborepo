import type { MigrateCommandOptions } from "../types";

const DEFAULT_REGISTRY = "https://registry.npmjs.org";

interface PackageDetailsResponse {
  "dist-tags": {
    latest: string;
    [key: string]: string;
  };
  versions: Record<string, { name: string; version: string }>;
}

async function getPackageDetails({ packageName }: { packageName: string }) {
  const registry =
    process.env.npm_config_registry?.replace(/\/$/, "") || DEFAULT_REGISTRY;

  try {
    const response = await fetch(`${registry}/${packageName}`);
    if (!response.ok) {
      throw new Error(`HTTP ${response.status}`);
    }
    return (await response.json()) as PackageDetailsResponse;
  } catch (err) {
    throw new Error(`Unable to fetch the latest version of ${packageName}`);
  }
}

export async function getLatestVersion({
  to
}: MigrateCommandOptions): Promise<string | undefined> {
  const packageDetails = await getPackageDetails({ packageName: "turbo" });
  const { "dist-tags": tags, versions } = packageDetails;

  if (to) {
    if (tags[to] || to in versions) {
      return to;
    }
    throw new Error(`turbo@${to} does not exist`);
  }

  return tags.latest;
}
