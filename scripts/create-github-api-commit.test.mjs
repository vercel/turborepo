import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import {
  buildFileChanges,
  parseArgs,
  run,
} from "./create-github-api-commit.mjs";

function git(args) {
  return execFileSync("git", args, { encoding: "utf8" }).trim();
}

async function withTempRepo(callback) {
  const cwd = process.cwd();
  const dir = mkdtempSync(join(tmpdir(), "github-api-commit-"));

  try {
    process.chdir(dir);
    git(["init"]);
    git(["checkout", "-b", "main"]);
    git(["config", "user.name", "Test User"]);
    git(["config", "user.email", "test@example.com"]);
    writeFileSync("file.txt", "base\n");
    git(["add", "file.txt"]);
    git(["commit", "-m", "initial"]);
    await callback(dir);
  } finally {
    process.chdir(cwd);
    rmSync(dir, { force: true, recursive: true });
  }
}

function mockGitHub({ commitOid = "commit-sha", remoteOid, verified = false }) {
  const requests = [];
  const originalFetch = globalThis.fetch;

  globalThis.fetch = async (url, options = {}) => {
    const body = options.body ? JSON.parse(options.body) : undefined;
    requests.push({ body, url: String(url) });

    if (String(url).endsWith("/graphql")) {
      if (body.query.includes("repository(owner:")) {
        return response({
          data: {
            repository: {
              id: "repo-id",
              nameWithOwner: "vercel/turbo",
              ref: remoteOid ? { target: { oid: remoteOid } } : null,
            },
          },
        });
      }
      if (body.query.includes("createRef")) {
        return response({ data: { createRef: { ref: { id: "ref-id" } } } });
      }
      if (body.query.includes("deleteRef")) {
        return response({ data: { deleteRef: { clientMutationId: null } } });
      }
      if (body.query.includes("createCommitOnBranch")) {
        return response({
          data: { createCommitOnBranch: { commit: { oid: commitOid } } },
        });
      }
    }

    return response({
      commit: { verification: { reason: "unsigned", verified } },
    });
  };

  return {
    requests,
    restore() {
      globalThis.fetch = originalFetch;
    },
  };
}

function response(data) {
  return { json: async () => data, ok: true, status: 200, statusText: "OK" };
}

test("parseArgs validates branch policy", () => {
  assert.deepEqual(
    parseArgs([
      "--branch",
      "release/test",
      "--message",
      "test",
      "--all-tracked",
      "--if-exists",
      "update",
    ]),
    {
      allTracked: true,
      branch: "release/test",
      ifExists: "update",
      includeUntracked: false,
      message: "test",
      paths: [],
    },
  );
  assert.throws(
    () =>
      parseArgs([
        "--branch",
        "release/test",
        "--message",
        "test",
        "--all-tracked",
        "--if-exists",
        "replace",
      ]),
    /--if-exists must be 'fail' or 'update'/,
  );
});

test("buildFileChanges refuses sensitive-looking paths", async () => {
  await withTempRepo(() => {
    writeFileSync(".env", "TOKEN=secret\n");
    assert.throws(
      () => buildFileChanges([{ deleted: false, path: ".env" }]),
      /Refusing to commit sensitive-looking file/,
    );
  });
});

test("verification failure preserves a branch after commit creation", async () => {
  await withTempRepo(async () => {
    writeFileSync("file.txt", "changed\n");
    process.env.GITHUB_REPOSITORY = "vercel/turbo";
    process.env.GH_TOKEN = "test-token";
    process.env.CREATE_GITHUB_API_COMMIT_VERIFY_ATTEMPTS = "1";
    process.env.CREATE_GITHUB_API_COMMIT_VERIFY_DELAY_MS = "0";

    const github = mockGitHub({ remoteOid: null, verified: false });
    try {
      await assert.rejects(
        run({
          allTracked: true,
          branch: "release/test",
          ifExists: "fail",
          includeUntracked: false,
          message: "test commit",
          paths: [],
        }),
        /GitHub did not verify commit commit-sha/,
      );
      assert.equal(
        github.requests.some((request) =>
          request.body?.query.includes("deleteRef"),
        ),
        false,
      );
    } finally {
      github.restore();
      delete process.env.GITHUB_REPOSITORY;
      delete process.env.GH_TOKEN;
      delete process.env.CREATE_GITHUB_API_COMMIT_VERIFY_ATTEMPTS;
      delete process.env.CREATE_GITHUB_API_COMMIT_VERIFY_DELAY_MS;
    }
  });
});

test("run reads changed files from repository root when invoked in a subdirectory", async () => {
  await withTempRepo(async () => {
    mkdirSync("cli");
    mkdirSync("packages/create-turbo", { recursive: true });
    writeFileSync("packages/create-turbo/package.json", '{"version":"1.0.0"}\n');
    git(["add", "cli", "packages/create-turbo/package.json"]);
    git(["commit", "-m", "add package"]);
    writeFileSync("packages/create-turbo/package.json", '{"version":"1.0.1"}\n');
    process.chdir("cli");
    const currentHead = git(["rev-parse", "HEAD"]);

    process.env.GITHUB_REPOSITORY = "vercel/turbo";
    process.env.GH_TOKEN = "test-token";
    process.env.CREATE_GITHUB_API_COMMIT_VERIFY_ATTEMPTS = "1";
    process.env.CREATE_GITHUB_API_COMMIT_VERIFY_DELAY_MS = "0";

    const github = mockGitHub({
      commitOid: currentHead,
      remoteOid: null,
      verified: true,
    });
    try {
      await run({
        allTracked: true,
        branch: "release/test",
        ifExists: "fail",
        includeUntracked: false,
        message: "test commit",
        paths: [],
      });

      const createCommitRequest = github.requests.find((request) =>
        request.body?.query.includes("createCommitOnBranch"),
      );
      assert.deepEqual(createCommitRequest.body.variables.input.fileChanges, {
        additions: [
          {
            contents: Buffer.from('{"version":"1.0.1"}\n').toString("base64"),
            path: "packages/create-turbo/package.json",
          },
        ],
      });
    } finally {
      github.restore();
      delete process.env.GITHUB_REPOSITORY;
      delete process.env.GH_TOKEN;
      delete process.env.CREATE_GITHUB_API_COMMIT_VERIFY_ATTEMPTS;
      delete process.env.CREATE_GITHUB_API_COMMIT_VERIFY_DELAY_MS;
    }
  });
});

test("update policy commits onto an existing remote branch", async () => {
  await withTempRepo(async () => {
    writeFileSync("file.txt", "changed\n");
    process.env.GITHUB_REPOSITORY = "vercel/turbo";
    process.env.GH_TOKEN = "test-token";
    process.env.CREATE_GITHUB_API_COMMIT_VERIFY_ATTEMPTS = "1";
    process.env.CREATE_GITHUB_API_COMMIT_VERIFY_DELAY_MS = "0";

    const github = mockGitHub({ remoteOid: "remote-sha", verified: false });
    try {
      await assert.rejects(
        run({
          allTracked: true,
          branch: "post-release-bump-examples",
          ifExists: "update",
          includeUntracked: false,
          message: "test commit",
          paths: [],
        }),
        /GitHub did not verify commit commit-sha/,
      );
      const createCommitRequest = github.requests.find((request) =>
        request.body?.query.includes("createCommitOnBranch"),
      );
      assert.equal(
        createCommitRequest.body.variables.input.expectedHeadOid,
        "remote-sha",
      );
    } finally {
      github.restore();
      delete process.env.GITHUB_REPOSITORY;
      delete process.env.GH_TOKEN;
      delete process.env.CREATE_GITHUB_API_COMMIT_VERIFY_ATTEMPTS;
      delete process.env.CREATE_GITHUB_API_COMMIT_VERIFY_DELAY_MS;
    }
  });
});

test("default policy refuses an existing remote branch", async () => {
  await withTempRepo(async () => {
    writeFileSync("file.txt", "changed\n");
    process.env.GITHUB_REPOSITORY = "vercel/turbo";
    process.env.GH_TOKEN = "test-token";

    const currentHead = git(["rev-parse", "HEAD"]);
    const github = mockGitHub({ remoteOid: currentHead, verified: false });
    try {
      await assert.rejects(
        run({
          allTracked: true,
          branch: "release/test",
          ifExists: "fail",
          includeUntracked: false,
          message: "test commit",
          paths: [],
        }),
        /Remote branch release\/test already exists/,
      );
      assert.equal(
        github.requests.some((request) =>
          request.body?.query.includes("createCommitOnBranch"),
        ),
        false,
      );
    } finally {
      github.restore();
      delete process.env.GITHUB_REPOSITORY;
      delete process.env.GH_TOKEN;
    }
  });
});
