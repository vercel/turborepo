#!/usr/bin/env node

import { Command } from "commander";
import { packAndPublish } from "./packager";
import { supportedPlatforms } from "./config";
import { getVersionInfo } from "./version";
import { publishRelease } from "./publish";

const turboReleaser = new Command();

turboReleaser
  .command("turbo", { isDefault: true })
  .requiredOption("--version-path <path>", "Path to the version.txt file")
  .option("--skip-publish", "Skip publishing to NPM")
  .action(releaseTurbo);

turboReleaser
  .command("publish")
  .requiredOption("--repo-root <path>", "Path to the repository root")
  .requiredOption(
    "--artifacts-dir <path>",
    "Directory containing dist-<os>-<arch> directories"
  )
  .requiredOption("--version-path <path>", "Path to the version.txt file")
  .option("--skip-publish", "Pack without publishing to npm")
  .action(publishRelease);

async function releaseTurbo(options: {
  skipPublish: boolean;
  versionPath: string;
}) {
  console.log("Command line options:", options);
  console.log("Supported platforms:", supportedPlatforms);

  try {
    const { version, npmTag } = await getVersionInfo(options.versionPath);
    console.log(`Using version: ${version}, NPM tag: ${npmTag}`);

    await packAndPublish({
      platforms: supportedPlatforms,
      version,
      skipPublish: options.skipPublish as boolean,
      npmTag,
      packagePrefix: "@turbo"
    });
    console.log("Packaging and publishing completed successfully");
  } catch (error) {
    console.error("Error during packaging and publishing:", error);
    process.exit(1);
  }
}

turboReleaser.parseAsync().catch((reason) => {
  console.error("Unexpected error. Please report it as a bug:", reason);
  process.exit(1);
});
