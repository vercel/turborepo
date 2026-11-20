import path from "node:path";
import { execFileSync } from "node:child_process";
import { readFile } from "node:fs/promises";
import { releasePackages } from "./config";
import { getVersionInfo } from "./version";

interface StageDependencies {
  run: (
    command: string,
    args: Array<string>,
    options: { cwd: string; stdio: "inherit" }
  ) => unknown;
  capture: (command: string, args: Array<string>, cwd: string) => string;
}

const defaultDependencies: StageDependencies = {
  run: (command, args, options) => execFileSync(command, args, options),
  capture: (command, args, cwd) =>
    execFileSync(command, args, { cwd, encoding: "utf8" })
};

export async function prepareStage({
  repoRoot,
  versionPath,
  dependencies = defaultDependencies
}: {
  repoRoot: string;
  versionPath: string;
  dependencies?: StageDependencies;
}) {
  const root = path.resolve(repoRoot);
  const resolvedVersionPath = path.resolve(root, versionPath);
  const relativeVersionPath = path.relative(root, resolvedVersionPath);
  const { version, npmTag } = await getVersionInfo(resolvedVersionPath);
  const branch = `staging-${version}`;

  console.log(`Version: ${version}`);
  console.log(`Tag: ${npmTag}`);
  console.log(await readFile(resolvedVersionPath, "utf8"));
  dependencies.run("git", ["status"], { cwd: root, stdio: "inherit" });

  if (
    !dependencies
      .capture("git", ["diff", "--", relativeVersionPath], root)
      .trim()
  ) {
    throw new Error("Refusing to publish with unupdated version.txt");
  }
  if (
    dependencies
      .capture(
        "git",
        ["ls-remote", "--tags", "origin", `refs/tags/v${version}`],
        root
      )
      .trim()
  ) {
    throw new Error(`Tag v${version} already exists`);
  }
  if (
    dependencies
      .capture(
        "git",
        ["ls-remote", "--heads", "origin", `refs/heads/${branch}`],
        root
      )
      .trim()
  ) {
    throw new Error(
      `Staging branch ${branch} already exists. If a previous release failed, re-run with clear-staging-branch enabled.`
    );
  }

  for (const releasePackage of releasePackages) {
    dependencies.run(
      "pnpm",
      [
        "version",
        version,
        "--allow-same-version",
        "--no-git-checks",
        "--no-git-tag-version"
      ],
      { cwd: path.join(root, releasePackage.directory), stdio: "inherit" }
    );
  }

  dependencies.run("git", ["checkout", "-b", branch], {
    cwd: root,
    stdio: "inherit"
  });

  return { branch, version };
}
