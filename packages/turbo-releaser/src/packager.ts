import type { Platform } from "./types";
import operations from "./operations";

interface PackAndPublishOptions {
  platforms: Array<Platform>;
  version: string;
  skipPublish: boolean;
  npmTag: string;
}

export async function packAndPublish({
  platforms,
  version,
  skipPublish,
  npmTag,
}: PackAndPublishOptions) {
  console.log("Starting packAndPublish process...");
  const artifacts: Array<string> = [];

  for (const platform of platforms) {
    console.log(`Processing platform: ${platform.os}-${platform.arch}`);
    // eslint-disable-next-line no-await-in-loop -- We trade of slightly faster releases with more legible logging
    const artifact = await operations.packPlatform({ platform, version });
    artifacts.push(artifact);
  }

  console.log("All platforms processed. Artifacts:", artifacts);

  if (!skipPublish) {
    console.log("Publishing artifacts...");
    operations.publishArtifacts(artifacts, npmTag);
  } else {
    console.log("Skipping publish step.");
  }
}
