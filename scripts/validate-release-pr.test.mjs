import assert from "node:assert/strict";
import test from "node:test";

import {
  CLI_PACKAGE_PATHS,
  LIBRARY_PACKAGE_PATHS,
  classifyRelease,
  run,
  validateReleaseFiles,
} from "./validate-release-pr.mjs";

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

function packageContent(name, version, optionalDependencies) {
  return `${JSON.stringify(
    {
      name,
      version,
      scripts: { test: "node test.js" },
      ...(optionalDependencies ? { optionalDependencies } : {}),
    },
    null,
    2,
  )}\n`;
}

function dependencies(names, version) {
  return Object.fromEntries(names.map((name) => [name, version]));
}

function cliFiles({
  baseVersion = "2.10.7-canary.1",
  version = "2.10.7-canary.2",
  versionFile = `${version}\ncanary\n`,
} = {}) {
  const files = CLI_PACKAGE_PATHS.map((path) => {
    const isTurbo = path === "packages/turbo/package.json";
    return {
      path,
      status: "modified",
      baseContent: packageContent(
        path,
        baseVersion,
        isTurbo ? dependencies(CLI_NATIVE_PACKAGES, baseVersion) : undefined,
      ),
      headContent: packageContent(
        path,
        version,
        isTurbo ? dependencies(CLI_NATIVE_PACKAGES, version) : undefined,
      ),
    };
  });
  files.push({
    path: "skills/turborepo/SKILL.md",
    status: "modified",
    baseContent: `---\nmetadata:\n  version: ${baseVersion}\n---\nhttps://turborepo.dev/schema.json\n`,
    headContent: `---\nmetadata:\n  version: ${version}\n---\nhttps://v${version.replace(/[.+]/g, "-")}.turborepo.dev/schema.json\n`,
  });
  files.push({
    path: "version.txt",
    status: "modified",
    baseContent: `${baseVersion}\ncanary\n`,
    headContent: versionFile,
  });
  return files;
}

function libraryFiles({
  baseVersion = "0.0.1-canary.22",
  version = "0.0.1-canary.23",
} = {}) {
  return LIBRARY_PACKAGE_PATHS.map((path) => {
    const isMain = path === "packages/turbo-repository/js/package.json";
    return {
      path,
      status: "modified",
      baseContent: packageContent(
        path,
        baseVersion,
        isMain ? dependencies(LIBRARY_NATIVE_PACKAGES, baseVersion) : undefined,
      ),
      headContent: packageContent(
        path,
        version,
        isMain ? dependencies(LIBRARY_NATIVE_PACKAGES, version) : undefined,
      ),
    };
  });
}

test("classifies exact CLI and library release titles", () => {
  assert.deepEqual(
    classifyRelease({
      headRef: "staging-2.10.7-canary.2",
      title: "chore: Release Turborepo 2.10.7-canary.2",
    }),
    { type: "cli", version: "2.10.7-canary.2" },
  );
  assert.deepEqual(
    classifyRelease({
      headRef: "library-release/0.0.1-canary.23",
      title: "chore: Release Turbo repository packages 0.0.1-canary.23",
    }),
    { type: "library", version: "0.0.1-canary.23" },
  );
});

test("rejects a title and branch version mismatch", () => {
  assert.throws(
    () =>
      classifyRelease({
        headRef: "staging-2.10.7-canary.2",
        title: "chore: Release Turborepo 2.10.7-canary.3",
      }),
    /title does not match/,
  );
});

test("accepts deterministic CLI prerelease changes", () => {
  assert.doesNotThrow(() =>
    validateReleaseFiles({
      release: { type: "cli", version: "2.10.7-canary.2" },
      files: cliFiles(),
    }),
  );
});

test("accepts the next canary version.txt after a stable release", () => {
  assert.doesNotThrow(() =>
    validateReleaseFiles({
      release: { type: "cli", version: "2.10.7" },
      files: cliFiles({
        version: "2.10.7",
        versionFile: "2.10.8-canary.0\ncanary\n",
      }),
    }),
  );
});

test("rejects package scripts and dependency changes", () => {
  for (const field of ["scripts", "dependencies"]) {
    const files = cliFiles();
    const target = files.find(
      ({ path }) => path === "packages/create-turbo/package.json",
    );
    const content = JSON.parse(target.headContent);
    content[field] = { malicious: "attacker.example/package" };
    target.headContent = JSON.stringify(content);

    assert.throws(
      () =>
        validateReleaseFiles({
          release: { type: "cli", version: "2.10.7-canary.2" },
          files,
        }),
      new RegExp(`/${field}`),
    );
  }
});

test("rejects missing, extra, and non-modified CLI files", () => {
  const release = { type: "cli", version: "2.10.7-canary.2" };
  assert.throws(
    () => validateReleaseFiles({ release, files: cliFiles().slice(1) }),
    /missing packages\/create-turbo/,
  );
  assert.throws(
    () =>
      validateReleaseFiles({
        release,
        files: [
          ...cliFiles(),
          {
            path: "Cargo.toml",
            status: "modified",
            baseContent: "",
            headContent: "",
          },
        ],
      }),
    /unexpected file Cargo.toml/,
  );
  const files = cliFiles();
  files[0].status = "added";
  assert.throws(
    () => validateReleaseFiles({ release, files }),
    /must be modified, not added/,
  );
});

test("rejects unrelated skill documentation edits", () => {
  const files = cliFiles();
  const skill = files.find(
    ({ path }) => path === "skills/turborepo/SKILL.md",
  );
  skill.headContent += "malicious instructions\n";
  assert.throws(
    () =>
      validateReleaseFiles({
        release: { type: "cli", version: "2.10.7-canary.2" },
        files,
      }),
    /not generated/,
  );
});

test("rejects an invalid stable release version.txt transition", () => {
  assert.throws(
    () =>
      validateReleaseFiles({
        release: { type: "cli", version: "2.10.7" },
        files: cliFiles({
          version: "2.10.7",
          versionFile: "2.10.7\nlatest\n",
        }),
      }),
    /must contain 2.10.8-canary.0/,
  );
});

test("accepts deterministic library release changes", () => {
  assert.doesNotThrow(() =>
    validateReleaseFiles({
      release: { type: "library", version: "0.0.1-canary.23" },
      files: libraryFiles(),
    }),
  );
});

test("rejects library optional dependency and file-set changes", () => {
  const files = libraryFiles();
  const main = files.find(
    ({ path }) => path === "packages/turbo-repository/js/package.json",
  );
  const content = JSON.parse(main.headContent);
  content.optionalDependencies["@turbo/repository-darwin-arm64"] =
    "0.0.1-canary.99";
  main.headContent = JSON.stringify(content);
  assert.throws(
    () =>
      validateReleaseFiles({
        release: { type: "library", version: "0.0.1-canary.23" },
        files,
      }),
    /must equal 0.0.1-canary.23/,
  );

  assert.throws(
    () =>
      validateReleaseFiles({
        release: { type: "library", version: "0.0.1-canary.23" },
        files: libraryFiles().slice(1),
      }),
    /unexpected set of files/,
  );
});

test("enumerates and reads release files only through immutable SHAs", async () => {
  const files = cliFiles();
  const filesByPath = new Map(files.map((file) => [file.path, file]));
  const requests = [];
  const originalFetch = globalThis.fetch;
  const envNames = [
    "GH_TOKEN",
    "GITHUB_REPOSITORY",
    "PR_BASE_SHA",
    "PR_HEAD_SHA",
    "PR_HEAD_REF",
    "PR_TITLE",
  ];
  const originalEnv = Object.fromEntries(
    envNames.map((name) => [name, process.env[name]]),
  );

  globalThis.fetch = async (url) => {
    const parsed = new URL(url);
    requests.push(parsed.pathname + parsed.search);
    if (parsed.pathname.includes("/compare/")) {
      return response({
        files: files.map(({ path, status }) => ({ filename: path, status })),
      });
    }

    const marker = "/contents/";
    const path = parsed.pathname
      .slice(parsed.pathname.indexOf(marker) + marker.length)
      .split("/")
      .map(decodeURIComponent)
      .join("/");
    const file = filesByPath.get(path);
    const content =
      parsed.searchParams.get("ref") === "base-sha"
        ? file.baseContent
        : file.headContent;
    return response({
      type: "file",
      encoding: "base64",
      content: Buffer.from(content).toString("base64"),
    });
  };
  Object.assign(process.env, {
    GH_TOKEN: "test-token",
    GITHUB_REPOSITORY: "vercel/turborepo",
    PR_BASE_SHA: "base-sha",
    PR_HEAD_SHA: "head-sha",
    PR_HEAD_REF: "staging-2.10.7-canary.2",
    PR_TITLE: "chore: Release Turborepo 2.10.7-canary.2",
  });

  try {
    await run();
  } finally {
    globalThis.fetch = originalFetch;
    for (const [name, value] of Object.entries(originalEnv)) {
      if (value === undefined) {
        delete process.env[name];
      } else {
        process.env[name] = value;
      }
    }
  }

  assert.equal(
    requests[0],
    "/repos/vercel/turborepo/compare/base-sha...head-sha",
  );
  assert.equal(requests.some((request) => request.includes("/pulls/")), false);
});

function response(data) {
  return {
    ok: true,
    async json() {
      return data;
    },
  };
}
