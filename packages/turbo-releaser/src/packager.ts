import { execSync } from "node:child_process";
import type { Platform } from "./types";
import operations from "./operations";

interface PackAndPublishOptions {
  platforms: Array<Platform>;
  version: string;
  skipPublish: boolean;
  npmTag: string;
  packagePrefix?: string;
  binaryName?: string;
  srcDirPrefix?: string;
  description?: string;
}

export async function packAndPublish({
  platforms,
  version,
  skipPublish,
  npmTag,
  packagePrefix,
  binaryName,
  srcDirPrefix,
  description
}: PackAndPublishOptions) {
  console.log("Starting packAndPublish process...");
  const artifacts: Array<string> = [];

  for (const platform of platforms) {
    console.log(`Processing platform: ${platform.os}-${platform.arch}`);
    // eslint-disable-next-line no-await-in-loop -- We trade of slightly faster releases with more legible logging
    const artifact = await operations.packPlatform({
      platform,
      version,
      packagePrefix,
      binaryName,
      srcDirPrefix,
      description
    });
    artifacts.push(artifact);
  }

  console.log("All platforms processed. Artifacts:", artifacts);

  if (!skipPublish) {
    console.log("Publishing artifacts...");
    const npmVersion = execSync("npm --version").toString().trim();
    console.log(`npm version: ${npmVersion}`);
    operations.publishArtifacts(artifacts, npmTag);
  } else {
    console.log("Skipping publish step.");
  }
}
