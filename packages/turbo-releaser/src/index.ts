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
  { os: "windows", arch: "arm64" }
];

// @turbo/gen doesn't have a windows-arm64 build (Bun doesn't support it)
const genPlatforms: Array<Platform> = [
  { os: "darwin", arch: "x64" },
  { os: "darwin", arch: "arm64" },
  { os: "linux", arch: "x64" },
  { os: "linux", arch: "arm64" },
  { os: "windows", arch: "x64" }
];

const turboReleaser = new Command();

turboReleaser
  .command("turbo", { isDefault: true })
  .requiredOption("--version-path <path>", "Path to the version.txt file")
  .option("--skip-publish", "Skip publishing to NPM")
  .action(releaseTurbo);

turboReleaser
  .command("gen")
  .description("Pack and publish @turbo/gen platform binaries")
  .requiredOption("--version-path <path>", "Path to the version.txt file")
  .option("--skip-publish", "Skip publishing to NPM")
  .action(releaseGen);

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
      npmTag
    });
    console.log("Packaging and publishing completed successfully");
  } catch (error) {
    console.error("Error during packaging and publishing:", error);
    process.exit(1);
  }
}

async function releaseGen(options: {
  skipPublish: boolean;
  versionPath: string;
}) {
  console.log("Releasing @turbo/gen platform packages...");
  console.log("Command line options:", options);

  try {
    const { version, npmTag } = await getVersionInfo(options.versionPath);
    console.log(`Using version: ${version}, NPM tag: ${npmTag}`);

    await packAndPublish({
      platforms: genPlatforms,
      version,
      skipPublish: options.skipPublish as boolean,
      npmTag,
      packagePrefix: "@turbo/gen",
      binaryName: "turbo-gen",
      srcDirPrefix: "dist-gen",
      description:
        "Platform binary for @turbo/gen, the Turborepo code generator."
    });
    console.log("@turbo/gen platform packages published successfully");
  } catch (error) {
    console.error("Error during @turbo/gen packaging:", error);
    process.exit(1);
  }
}

turboReleaser.parseAsync().catch((reason) => {
  console.error("Unexpected error. Please report it as a bug:", reason);
  process.exit(1);
});
