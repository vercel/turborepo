#!/usr/bin/env node

import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { basename, dirname, join, resolve } from "node:path";
import { pathToFileURL } from "node:url";

export const STANDALONE_TARGETS = [
  { triple: "x86_64-apple-darwin", executable: "turbo" },
  { triple: "aarch64-apple-darwin", executable: "turbo" },
  { triple: "x86_64-unknown-linux-musl", executable: "turbo" },
  { triple: "aarch64-unknown-linux-musl", executable: "turbo" },
  { triple: "x86_64-pc-windows-msvc", executable: "turbo.exe" }
];

const VERSION_PATTERN = /^[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.]+)?$/;

export function archiveName(version, triple) {
  if (!VERSION_PATTERN.test(version)) {
    throw new Error(`Invalid release version: ${version}`);
  }
  return `turbo-${version}-${triple}.tar.gz`;
}

export function archiveMembers(archivePath) {
  const output = execFileSync("tar", ["-tzf", archivePath], {
    encoding: "utf8"
  });
  return output.trimEnd().split(/\r?\n/).filter(Boolean);
}

export function assertArchiveLayout(archivePath, executable) {
  const members = archiveMembers(archivePath);
  if (members.length !== 1 || members[0] !== executable) {
    throw new Error(
      `${basename(archivePath)} must contain only ${executable} at its root; found ${members.join(", ") || "no files"}`
    );
  }
}

export function packageStandaloneArchives({
  version,
  artifactsDirectory,
  outputDirectory
}) {
  if (!VERSION_PATTERN.test(version)) {
    throw new Error(`Invalid release version: ${version}`);
  }

  const artifactRoot = resolve(artifactsDirectory);
  const archiveRoot = resolve(outputDirectory);
  mkdirSync(archiveRoot, { recursive: true });
  const archives = STANDALONE_TARGETS.map(({ triple, executable }) => {
    const binaryPath = join(artifactRoot, `turbo-${triple}`, executable);
    if (!existsSync(binaryPath)) {
      throw new Error(`Missing release binary for ${triple}: ${binaryPath}`);
    }

    const archivePath = join(archiveRoot, archiveName(version, triple));
    execFileSync(
      "tar",
      ["-czf", archivePath, "-C", dirname(binaryPath), executable],
      { stdio: "inherit" }
    );
    assertArchiveLayout(archivePath, executable);
    console.log(`Created ${archivePath}`);
    return archivePath;
  });

  const checksumLines = archives.map((archivePath) => {
    const digest = createHash("sha256")
      .update(readFileSync(archivePath))
      .digest("hex");
    return `${digest}  ${basename(archivePath)}`;
  });
  writeFileSync(
    join(archiveRoot, "SHA256SUMS"),
    `${checksumLines.join("\n")}\n`
  );
  return archives;
}

if (
  process.argv[1] &&
  pathToFileURL(resolve(process.argv[1])).href === import.meta.url
) {
  const [
    version,
    artifactsDirectory = "rust-artifacts",
    outputDirectory = "standalone-artifacts"
  ] = process.argv.slice(2);
  if (!version) {
    console.error(
      "Usage: package-standalone-archives.mjs <version> [artifacts-directory] [output-directory]"
    );
    process.exitCode = 1;
  } else {
    try {
      packageStandaloneArchives({
        version,
        artifactsDirectory,
        outputDirectory
      });
    } catch (error) {
      console.error(error.message);
      process.exitCode = 1;
    }
  }
}
