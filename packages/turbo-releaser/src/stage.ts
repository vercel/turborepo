import path from "node:path";
import { execFileSync } from "node:child_process";
import { readFile, readdir, writeFile } from "node:fs/promises";
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
      ["version", version, "--allow-same-version", "--no-git-tag-version"],
      { cwd: path.join(root, releasePackage.directory), stdio: "inherit" }
    );
  }

  await updateSkillFiles(path.join(root, "skills", "turborepo"), version);
  dependencies.run("git", ["checkout", "-b", branch], {
    cwd: root,
    stdio: "inherit"
  });

  return { branch, version };
}

async function updateSkillFiles(skillRoot: string, version: string) {
  const skillPath = path.join(skillRoot, "SKILL.md");
  const skill = await readFile(skillPath, "utf8");
  await writeFile(
    skillPath,
    skill.replace(
      /^(---\n[\s\S]*?metadata:\n\s*version:\s*).+?(\n[\s\S]*?---)/,
      `$1${version}$2`
    )
  );

  const schemaUrl = `https://v${version.replace(/[.+]/g, "-")}.turborepo.dev/schema.json`;
  const schemaPattern =
    /https:\/\/(?:v[\w-]+\.)?turborepo\.(?:dev|com)\/schema(?:\.v2)?\.json|https:\/\/turbo\.build\/schema(?:\.v2)?\.json/g;

  for (const file of await markdownFiles(skillRoot)) {
    const contents = await readFile(file, "utf8");
    const updated = contents.replace(schemaPattern, schemaUrl);
    if (updated !== contents) {
      await writeFile(file, updated);
    }
  }
}

async function markdownFiles(directory: string): Promise<Array<string>> {
  const files: Array<string> = [];
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    const entryPath = path.join(directory, entry.name);
    if (entry.isDirectory()) {
      files.push(...(await markdownFiles(entryPath)));
    } else if (entry.name.endsWith(".md")) {
      files.push(entryPath);
    }
  }
  return files;
}
