import { Command } from "commander";
import { packAndPublish } from "./packager";
import type { Platform } from "./types";
import { getVersionInfo } from "./version";

const supportedPlatforms: Array<Platform> = [
  { os: "darwin", arch: "x64" },
  { os: "darwin", arch: "arm64" },
  { os: "linux", arch: "x64" },
  { os: "linux", arch: "arm64" },
  { os: "windows", arch: "x64" },
  { os: "windows", arch: "arm64" },
];

const turboReleaser = new Command();
turboReleaser
  .requiredOption("--version-path <path>", "Path to the version.txt file")
  .option("--skip-publish", "Skip publishing to NPM")
  .action(main);

async function main(options: { skipPublish: boolean; versionPath: string }) {
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
