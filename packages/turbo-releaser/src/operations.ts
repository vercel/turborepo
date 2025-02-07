import path from "node:path";
import fs from "node:fs/promises";
import { execSync } from "node:child_process";
import tar from "tar";
import native from "./native";
import type { Platform } from "./types";

export interface PackOptions {
  platform: Platform;
  version: string;
  // Directory where prebuilt `turbo` binaries are located
  // It will also be where `dist` is created and tarballs are put.
  // Defaults to cwd
  srcDir?: string;
}

async function packPlatform({
  platform,
  version,
  srcDir = process.cwd(),
}: PackOptions): Promise<string> {
  const { os, arch } = platform;
  console.log(`Packing platform: ${os}-${arch}`);
  const npmDirName = `turbo-${os}-${arch}`;
  const tarballDir = path.join(srcDir, "dist", `${os}-${arch}-${version}`);
  const scaffoldDir = path.join(tarballDir, npmDirName);

  console.log("Generating native package...");
  await native.generateNativePackage({
    platform,
    version,
    outputDir: scaffoldDir,
  });

  console.log("Moving prebuilt binary...");
  const binaryName = os === "windows" ? "turbo.exe" : "turbo";
  const sourcePath = path.join(srcDir, `dist-${os}-${arch}`, binaryName);
  const destPath = path.join(scaffoldDir, "bin", binaryName);
  await fs.mkdir(path.dirname(destPath), { recursive: true });
  await fs.copyFile(sourcePath, destPath);
  // Make sure the binary we copied is executable
  const stat = await fs.stat(destPath);
  const currMode = stat.mode;
  // eslint-disable-next-line no-bitwise -- necessary for enabling the executable bits
  await fs.chmod(destPath, currMode | 0o111);

  console.log("Creating tar.gz...");
  const tarName = `${os}-${arch}-${version}.tar.gz`;
  const tarPath = path.join(srcDir, "dist", tarName);
  await tar.create(
    {
      gzip: true,
      file: tarPath,
      cwd: tarballDir,
    },
    [npmDirName]
  );

  console.log(`Artifact created: ${tarPath}`);
  return path.resolve(tarPath);
}

function publishArtifacts(artifacts: Array<string>, npmTag: string) {
  for (const artifact of artifacts) {
    const publishCommand = `npm publish "${artifact}" --tag ${npmTag}`;
    console.log(`Executing: ${publishCommand}`);
    execSync(publishCommand, { stdio: "inherit" });
  }
}

export default { packPlatform, publishArtifacts };
