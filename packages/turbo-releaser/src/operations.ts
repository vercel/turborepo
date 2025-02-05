import path from "node:path";
import fs from "node:fs/promises";
import { execSync } from "node:child_process";
import tar from "tar";
import native from "./native";
import type { Platform } from "./types";

async function packPlatform(
  platform: Platform,
  version: string
): Promise<string> {
  const { os, arch } = platform;
  console.log(`Packing platform: ${os}-${arch}`);
  const scaffoldDir = path.join("dist", `${os}-${arch}-${version}`);

  console.log("Generating native package...");
  await native.generateNativePackage({
    platform,
    version,
    outputDir: scaffoldDir,
  });

  console.log("Moving prebuilt binary...");
  const binaryName = os === "windows" ? "turbo.exe" : "turbo";
  const sourcePath = path.join(`dist-${os}-${arch}`, binaryName);
  const destPath = path.join(scaffoldDir, "bin", binaryName);
  await fs.mkdir(path.dirname(destPath), { recursive: true });
  await fs.copyFile(sourcePath, destPath);

  console.log("Creating tar.gz...");
  const tarName = `${os}-${arch}-${version}.tar.gz`;
  const tarPath = path.join("dist", tarName);
  await tar.create(
    {
      gzip: true,
      file: tarPath,
      cwd: scaffoldDir,
    },
    ["package.json", "README.md", "bin"]
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
