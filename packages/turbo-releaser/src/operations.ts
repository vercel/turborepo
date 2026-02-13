import path from "node:path";
import fs from "node:fs/promises";
import { execSync } from "node:child_process";
import * as tar from "tar";
import native from "./native";
import type { Platform } from "./types";

export interface PackOptions {
  platform: Platform;
  version: string;
  // Directory where prebuilt binaries are located.
  // It will also be where `dist` is created and tarballs are put.
  // Defaults to cwd
  srcDir?: string;
  // Package name prefix (e.g., "turbo" for turbo-darwin-arm64, "@turbo/gen" for @turbo/gen-darwin-arm64)
  packagePrefix?: string;
  // Binary name (e.g., "turbo" or "turbo-gen")
  binaryName?: string;
  // Prefix for source binary directories (e.g., "dist" → "dist-darwin-arm64", "dist-gen" → "dist-gen-darwin-arm64")
  srcDirPrefix?: string;
  // Description override for the generated package.json
  description?: string;
}

async function packPlatform({
  platform,
  version,
  srcDir = process.cwd(),
  packagePrefix = "turbo",
  binaryName: binaryBaseName = "turbo",
  srcDirPrefix = "dist",
  description
}: PackOptions): Promise<string> {
  const { os, arch } = platform;
  console.log(`Packing platform: ${os}-${arch}`);
  const npmDirName = `${packagePrefix}-${os}-${arch}`
    .replace("@", "")
    .replace("/", "-");
  const tarballDir = path.join(srcDir, "dist", `${npmDirName}-${version}`);
  const scaffoldDir = path.join(tarballDir, npmDirName);

  console.log("Generating native package...");
  await native.generateNativePackage({
    platform,
    version,
    outputDir: scaffoldDir,
    packagePrefix,
    description
  });

  console.log("Moving prebuilt binary...");
  const binaryName =
    os === "windows" ? `${binaryBaseName}.exe` : binaryBaseName;
  const sourcePath = path.join(
    srcDir,
    `${srcDirPrefix}-${os}-${arch}`,
    binaryName
  );
  const destPath = path.join(scaffoldDir, "bin", binaryName);
  await fs.mkdir(path.dirname(destPath), { recursive: true });
  await fs.copyFile(sourcePath, destPath);
  const stat = await fs.stat(destPath);
  const currMode = stat.mode;
  // eslint-disable-next-line no-bitwise -- necessary for enabling the executable bits
  await fs.chmod(destPath, currMode | 0o111);

  console.log("Creating tar.gz...");
  const tarName = `${npmDirName}-${version}.tar.gz`;
  const tarPath = path.join(srcDir, "dist", tarName);
  await tar.create(
    {
      gzip: true,
      file: tarPath,
      cwd: tarballDir
    },
    [npmDirName]
  );

  console.log(`Artifact created: ${tarPath}`);
  return path.resolve(tarPath);
}

function publishArtifacts(artifacts: Array<string>, npmTag: string) {
  for (const artifact of artifacts) {
    const npmVersion = execSync("npm --version").toString().trim();
    console.log(`npm version: ${npmVersion}`);
    const publishCommand = `npm publish "${artifact}" --tag ${npmTag}`;
    console.log(`Executing: ${publishCommand}`);
    execSync(publishCommand, { stdio: "inherit" });
  }
}

export default { packPlatform, publishArtifacts };
