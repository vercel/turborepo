#!/usr/bin/env node

export const CLI_PACKAGE_PATHS = [
  "packages/create-turbo/package.json",
  "packages/eslint-config-turbo/package.json",
  "packages/eslint-plugin-turbo/package.json",
  "packages/turbo-codemod/package.json",
  "packages/turbo-gen/package.json",
  "packages/turbo-ignore/package.json",
  "packages/turbo-types/package.json",
  "packages/turbo-workspaces/package.json",
  "packages/turbo/package.json",
];

export const LIBRARY_PACKAGE_PATHS = [
  "packages/turbo-repository/js/package.json",
  "packages/turbo-repository/npm/darwin-arm64/package.json",
  "packages/turbo-repository/npm/darwin-x64/package.json",
  "packages/turbo-repository/npm/linux-arm64-gnu/package.json",
  "packages/turbo-repository/npm/linux-arm64-musl/package.json",
  "packages/turbo-repository/npm/linux-x64-gnu/package.json",
  "packages/turbo-repository/npm/linux-x64-musl/package.json",
  "packages/turbo-repository/npm/win32-arm64-msvc/package.json",
  "packages/turbo-repository/npm/win32-x64-msvc/package.json",
];

const CLI_NATIVE_PACKAGES = [
  "@turbo/darwin-64",
  "@turbo/darwin-arm64",
  "@turbo/linux-64",
  "@turbo/linux-arm64",
  "@turbo/windows-64",
  "@turbo/windows-arm64",
];

const LIBRARY_NATIVE_PACKAGES = [
  "@turbo/repository-darwin-arm64",
  "@turbo/repository-darwin-x64",
  "@turbo/repository-linux-arm64-gnu",
  "@turbo/repository-linux-arm64-musl",
  "@turbo/repository-linux-x64-gnu",
  "@turbo/repository-linux-x64-musl",
  "@turbo/repository-win32-arm64-msvc",
  "@turbo/repository-win32-x64-msvc",
];

const VERSION_PATTERN = /^[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.]+)?$/;
const SKILL_PATH_PATTERN = /^skills\/turborepo\/.*\.md$/;
const SCHEMA_PATTERN =
  /https:\/\/(?:v[\w-]+\.)?turborepo\.(?:dev|com)\/schema(?:\.v2)?\.json|https:\/\/turbo\.build\/schema(?:\.v2)?\.json/g;

export function classifyRelease({ headRef, title }) {
  const cliMatch = /^staging-(.+)$/.exec(headRef);
  if (cliMatch && VERSION_PATTERN.test(cliMatch[1])) {
    const version = cliMatch[1];
    if (title !== `chore: Release Turborepo ${version}`) {
      throw new Error("CLI release title does not match its staging branch");
    }
    return { type: "cli", version };
  }

  const libraryMatch = /^library-release\/(.+)$/.exec(headRef);
  if (libraryMatch && VERSION_PATTERN.test(libraryMatch[1])) {
    const version = libraryMatch[1];
    if (title !== `chore: Release Turbo repository packages ${version}`) {
      throw new Error("Library release title does not match its release branch");
    }
    return { type: "library", version };
  }

  throw new Error(`Unrecognized release branch: ${headRef}`);
}

function jsonPointerSegment(value) {
  return value.replaceAll("~", "~0").replaceAll("/", "~1");
}

function changedJsonPaths(base, head, prefix = "") {
  if (Object.is(base, head)) {
    return [];
  }
  if (
    base === null ||
    head === null ||
    typeof base !== "object" ||
    typeof head !== "object" ||
    Array.isArray(base) !== Array.isArray(head)
  ) {
    return [prefix || "/"];
  }

  const keys = new Set([...Object.keys(base), ...Object.keys(head)]);
  return [...keys].flatMap((key) =>
    changedJsonPaths(
      base[key],
      head[key],
      `${prefix}/${jsonPointerSegment(key)}`,
    ),
  );
}

function valueAtPointer(value, pointer) {
  return pointer
    .slice(1)
    .split("/")
    .reduce(
      (current, part) =>
        current?.[part.replaceAll("~1", "/").replaceAll("~0", "~")],
      value,
    );
}

function validatePackageJson({
  path,
  baseContent,
  headContent,
  version,
  versionedDependencies = [],
}) {
  let base;
  let head;
  try {
    base = JSON.parse(baseContent);
    head = JSON.parse(headContent);
  } catch {
    throw new Error(`${path} must contain valid JSON`);
  }

  const allowedPaths = [
    "/version",
    ...versionedDependencies.map(
      (name) => `/optionalDependencies/${jsonPointerSegment(name)}`,
    ),
  ].sort();
  const changedPaths = changedJsonPaths(base, head).sort();

  if (JSON.stringify(changedPaths) !== JSON.stringify(allowedPaths)) {
    throw new Error(
      `${path} changes unexpected fields: ${changedPaths.join(", ")}`,
    );
  }
  for (const pointer of allowedPaths) {
    if (valueAtPointer(head, pointer) !== version) {
      throw new Error(`${path}${pointer} must equal ${version}`);
    }
  }
}

function nextPatchCanary(version) {
  const [major, minor, patch] = version.split(".").map(Number);
  return `${major}.${minor}.${patch + 1}-canary.0`;
}

function validateVersionFile(content, version) {
  const match = /^([^\n]+)\n([^\n]+)\n$/.exec(content);
  if (!match) {
    throw new Error("version.txt must contain exactly a version and npm tag");
  }

  const expectedVersion = version.includes("-")
    ? version
    : nextPatchCanary(version);
  if (match[1] !== expectedVersion) {
    throw new Error(`version.txt must contain ${expectedVersion}`);
  }
  if (!/^[0-9A-Za-z][0-9A-Za-z._-]{0,127}$/.test(match[2])) {
    throw new Error("version.txt contains an invalid npm tag");
  }
  if (!version.includes("-") && match[2] !== "canary") {
    throw new Error("A stable release must advance version.txt to canary");
  }
}

function validateSkillFile({ path, baseContent, headContent, version }) {
  let expected = baseContent;
  if (path === "skills/turborepo/SKILL.md") {
    expected = expected.replace(
      /^(---\n[\s\S]*?metadata:\n\s*version:\s*).+?(\n[\s\S]*?---)/,
      `$1${version}$2`,
    );
  }

  const schemaUrl = `https://v${version.replace(/[.+]/g, "-")}.turborepo.dev/schema.json`;
  expected = expected.replace(SCHEMA_PATTERN, schemaUrl);
  if (expected === baseContent || headContent !== expected) {
    throw new Error(`${path} contains changes not generated for ${version}`);
  }
}

export function validateReleaseFiles({ release, files }) {
  const filesByPath = new Map(files.map((file) => [file.path, file]));
  if (filesByPath.size !== files.length) {
    throw new Error("Release PR contains duplicate file entries");
  }
  for (const file of files) {
    if (file.status !== "modified") {
      throw new Error(`${file.path} must be modified, not ${file.status}`);
    }
  }

  if (release.type === "library") {
    const actualPaths = [...filesByPath.keys()].sort();
    const expectedPaths = [...LIBRARY_PACKAGE_PATHS].sort();
    if (JSON.stringify(actualPaths) !== JSON.stringify(expectedPaths)) {
      throw new Error("Library release contains an unexpected set of files");
    }

    for (const path of LIBRARY_PACKAGE_PATHS) {
      const file = filesByPath.get(path);
      validatePackageJson({
        path,
        baseContent: file.baseContent,
        headContent: file.headContent,
        version: release.version,
        versionedDependencies:
          path === "packages/turbo-repository/js/package.json"
            ? LIBRARY_NATIVE_PACKAGES
            : [],
      });
    }
    return;
  }

  for (const path of CLI_PACKAGE_PATHS) {
    if (!filesByPath.has(path)) {
      throw new Error(`CLI release is missing ${path}`);
    }
  }
  if (!filesByPath.has("version.txt")) {
    throw new Error("CLI release is missing version.txt");
  }
  if (!filesByPath.has("skills/turborepo/SKILL.md")) {
    throw new Error("CLI release is missing skills/turborepo/SKILL.md");
  }

  for (const file of files) {
    if (CLI_PACKAGE_PATHS.includes(file.path)) {
      validatePackageJson({
        path: file.path,
        baseContent: file.baseContent,
        headContent: file.headContent,
        version: release.version,
        versionedDependencies:
          file.path === "packages/turbo/package.json"
            ? CLI_NATIVE_PACKAGES
            : [],
      });
    } else if (file.path === "version.txt") {
      validateVersionFile(file.headContent, release.version);
    } else if (SKILL_PATH_PATTERN.test(file.path)) {
      validateSkillFile({ ...file, version: release.version });
    } else {
      throw new Error(`CLI release contains unexpected file ${file.path}`);
    }
  }
}

function requiredEnv(name) {
  const value = process.env[name];
  if (!value) {
    throw new Error(`Missing ${name}`);
  }
  return value;
}

async function github(path) {
  const response = await fetch(`https://api.github.com${path}`, {
    headers: {
      Accept: "application/vnd.github+json",
      Authorization: `Bearer ${requiredEnv("GH_TOKEN")}`,
      "X-GitHub-Api-Version": "2022-11-28",
    },
  });
  if (!response.ok) {
    throw new Error(`GitHub API request failed (${response.status}): ${path}`);
  }
  return response.json();
}

async function changedFiles(repository, baseSha, headSha) {
  const comparison = await github(
    `/repos/${repository}/compare/${encodeURIComponent(baseSha)}...${encodeURIComponent(headSha)}`,
  );
  if (!Array.isArray(comparison.files) || comparison.files.length >= 300) {
    throw new Error("Unable to enumerate the complete immutable release diff");
  }
  return comparison.files;
}

async function fileContent(repository, path, sha) {
  const encodedPath = path.split("/").map(encodeURIComponent).join("/");
  const data = await github(
    `/repos/${repository}/contents/${encodedPath}?ref=${encodeURIComponent(sha)}`,
  );
  if (data.type !== "file" || data.encoding !== "base64") {
    throw new Error(`Unable to read ${path} at ${sha}`);
  }
  return Buffer.from(data.content, "base64").toString("utf8");
}

export async function run() {
  const repository = requiredEnv("GITHUB_REPOSITORY");
  const baseSha = requiredEnv("PR_BASE_SHA");
  const headSha = requiredEnv("PR_HEAD_SHA");
  const release = classifyRelease({
    headRef: requiredEnv("PR_HEAD_REF"),
    title: requiredEnv("PR_TITLE"),
  });
  const apiFiles = await changedFiles(repository, baseSha, headSha);
  const files = await Promise.all(
    apiFiles.map(async ({ filename: path, status }) => ({
      path,
      status,
      baseContent: await fileContent(repository, path, baseSha),
      headContent: await fileContent(repository, path, headSha),
    })),
  );

  validateReleaseFiles({ release, files });
  console.log(`Validated ${release.type} release ${release.version}`);
}

if (import.meta.url === `file://${process.argv[1]}`) {
  run().catch((error) => {
    console.error(`::error::${error.message}`);
    process.exitCode = 1;
  });
}
