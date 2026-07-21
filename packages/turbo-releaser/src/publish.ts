import path from "node:path";
import { execFileSync } from "node:child_process";
import { packAndPublish } from "./packager";
import {
  releaseBuildFilters,
  releasePackages,
  supportedPlatforms
} from "./config";
import { getVersionInfo } from "./version";
import { publishWithRetries } from "./npm";

interface PublishReleaseDependencies {
  run: (
    command: string,
    args: Array<string>,
    options: { cwd: string; stdio: "ignore" | "inherit" }
  ) => unknown;
  packAndPublish: typeof packAndPublish;
  publishWithRetries: typeof publishWithRetries;
}

const defaultDependencies: PublishReleaseDependencies = {
  run: (command, args, options) => execFileSync(command, args, options),
  packAndPublish,
  publishWithRetries
};

export async function publishRelease({
  repoRoot,
  artifactsDir,
  versionPath,
  skipPublish,
  dependencies = defaultDependencies
}: {
  repoRoot: string;
  artifactsDir: string;
  versionPath: string;
  skipPublish: boolean;
  dependencies?: PublishReleaseDependencies;
}) {
  const root = path.resolve(repoRoot);
  const artifacts = path.resolve(root, artifactsDir);
  const { version, npmTag } = await getVersionInfo(
    path.resolve(root, versionPath)
  );

  dependencies.run(
    "turbo",
    [
      "run",
      "build",
      "copy-schema",
      ...releaseBuildFilters.map((name) => `--filter=${name}`)
    ],
    { cwd: root, stdio: "inherit" }
  );

  dependencies.run("git", ["format-patch", "HEAD~1", "--stdout"], {
    cwd: root,
    stdio: "inherit"
  });

  await dependencies.packAndPublish({
    platforms: supportedPlatforms,
    version,
    skipPublish,
    npmTag,
    packagePrefix: "@turbo",
    srcDir: artifacts
  });

  for (const releasePackage of releasePackages) {
    dependencies.run("pnpm", ["pack", `--pack-destination=${root}`], {
      cwd: path.join(root, releasePackage.directory),
      stdio: "inherit"
    });
  }

  if (skipPublish) {
    return;
  }

  const existingVersion = npmVersionExists(version, root, dependencies.run);
  if (existingVersion) {
    throw new Error(
      `turbo@${version} already exists on npm. A previous release may still be merging.`
    );
  }

  for (const releasePackage of releasePackages) {
    await dependencies.publishWithRetries({
      packageName: `${releasePackage.name}@${version}`,
      tarball: path.join(root, `${releasePackage.tarball}-${version}.tgz`),
      npmTag
    });
  }
}

function npmVersionExists(
  version: string,
  cwd: string,
  run: PublishReleaseDependencies["run"]
): boolean {
  try {
    run("npm", ["view", `turbo@${version}`, "version"], {
      cwd,
      stdio: "ignore"
    });
    return true;
  } catch {
    return false;
  }
}
