import assert from "node:assert/strict";
import test from "node:test";

import {
  installWorkspacePublishCommand,
  isWorkspacePublishRequest,
  parseWorkspacePublishInput,
  workspacePublishBridge
} from "../agent/lib/workspace-publish.ts";

const workspace = {
  id: "ws_abc",
  publishToken: "secret-token"
};

test("validates workspace publication metadata", () => {
  assert.deepEqual(
    parseWorkspacePublishInput({
      body: "Tests: pnpm test",
      branchName: "agents/fix-cache",
      title: "fix: Repair cache lookup"
    }),
    {
      body: "Tests: pnpm test",
      branchName: "agents/fix-cache",
      title: "fix: Repair cache lookup"
    }
  );
  assert.equal(
    parseWorkspacePublishInput({
      body: "",
      branchName: "main",
      title: "fix: Repair cache lookup"
    }),
    null
  );
  assert.equal(
    parseWorkspacePublishInput({
      body: "",
      branchName: "agents/fix-cache",
      title: "fix: lowercase"
    }),
    null
  );
});

test("installs the Factory publishing skill for fx", async () => {
  const writes = [];
  const commands = [];
  const sandbox = {
    async runCommand(command) {
      commands.push(command);
      return { exitCode: 0 };
    },
    async writeFiles(files) {
      writes.push(...files);
    }
  };

  await installWorkspacePublishCommand(sandbox, {
    authorization: "Bearer secret-token",
    hostname: "factory.example",
    path: "/api/workspaces/ws_abc/publish",
    url: "https://factory.example/api/workspaces/ws_abc/publish"
  });

  const skill = writes.find(({ path }) =>
    path.endsWith("/skills/factory-publish/SKILL.md")
  );
  assert.ok(skill);
  const contents = skill.content.toString("utf8");
  assert.match(contents, /name: factory-publish/);
  assert.match(contents, /create, make, open, or publish a pull request/);
  assert.match(contents, /factory-create-pr/);
  assert.match(contents, /Never run `git commit`/);
  assert.deepEqual(commands.at(-1).args, [
    "+x",
    "/factory/bin/factory-create-pr"
  ]);
});

test("requires the private workspace publication capability", () => {
  assert.equal(
    isWorkspacePublishRequest(
      new Request("https://factory.example", {
        headers: { authorization: "Bearer secret-token" }
      }),
      workspace
    ),
    true
  );
  assert.equal(
    isWorkspacePublishRequest(
      new Request("https://factory.example", {
        headers: { authorization: "Bearer wrong-token" }
      }),
      workspace
    ),
    false
  );
});

test("builds a deployment-local publication bridge", () => {
  const previous = process.env.VERCEL_URL;
  process.env.VERCEL_URL = "factory.example";
  try {
    assert.deepEqual(workspacePublishBridge("ws_abc", "secret-token"), {
      authorization: "Bearer secret-token",
      hostname: "factory.example",
      path: "/api/workspaces/ws_abc/publish",
      url: "https://factory.example/api/workspaces/ws_abc/publish"
    });
  } finally {
    if (previous === undefined) delete process.env.VERCEL_URL;
    else process.env.VERCEL_URL = previous;
  }
});
