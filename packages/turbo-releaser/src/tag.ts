import path from "node:path";
import { execFileSync } from "node:child_process";
import { getVersionInfo } from "./version";

interface TagDependencies {
  run: (command: string, args: Array<string>, cwd: string) => unknown;
  capture: (command: string, args: Array<string>, cwd: string) => string;
}

const defaultDependencies: TagDependencies = {
  run: (command, args, cwd) =>
    execFileSync(command, args, { cwd, stdio: "inherit" }),
  capture: (command, args, cwd) =>
    execFileSync(command, args, { cwd, encoding: "utf8" })
};

export async function createReleaseTag({
  repoRoot,
  versionPath,
  dependencies = defaultDependencies
}: {
  repoRoot: string;
  versionPath: string;
  dependencies?: TagDependencies;
}) {
  const root = path.resolve(repoRoot);
  const { version } = await getVersionInfo(path.resolve(root, versionPath));
  const tag = `v${version}`;
  const remoteSha = dependencies
    .capture("git", ["ls-remote", "--tags", "origin", `refs/tags/${tag}`], root)
    .trim()
    .split(/\s+/)[0];
  const localSha = dependencies
    .capture("git", ["rev-parse", "HEAD"], root)
    .trim();

  if (remoteSha === localSha) {
    console.log(`Tag ${tag} already exists at the correct commit. Skipping.`);
    return;
  }
  if (remoteSha) {
    console.log(`Replacing ${tag} at ${remoteSha}; expected ${localSha}.`);
    dependencies.run("git", ["push", "origin", `:refs/tags/${tag}`], root);
    dependencies.run("git", ["tag", "-f", tag], root);
  } else {
    dependencies.run("git", ["tag", tag], root);
  }
  dependencies.run("git", ["push", "origin", tag], root);
}
