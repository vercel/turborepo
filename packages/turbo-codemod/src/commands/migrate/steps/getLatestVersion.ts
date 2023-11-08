import axios from "axios";
import type { MigrateCommandOptions } from "../types";

const REGISTRY = "https://registry.npmjs.org";

interface PackageDetailsResponse {
  "dist-tags": {
    latest: string;
    [key: string]: string;
  };
  versions: Record<string, { name: string; version: string }>;
}

async function getPackageDetails({ packageName }: { packageName: string }) {
  try {
    const result = await axios.get<PackageDetailsResponse>(
      `${REGISTRY}/${packageName}`
    );
    return result.data;
  } catch (err) {
    throw new Error(`Unable to fetch the latest version of ${packageName}`);
  }
}

export async function getLatestVersion({
  to,
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
