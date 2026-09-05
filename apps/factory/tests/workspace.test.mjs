import assert from "node:assert/strict";
import test from "node:test";

import {
  isWorkspaceHarness,
  isWorkspaceMutationRequest,
  isWorkspaceModel,
  isWorkspaceRecord,
  parseCreateWorkspaceInput,
  DEFAULT_WORKSPACE_HARNESS,
  DEFAULT_WORKSPACE_MODEL,
  toWorkspaceSummary,
  WORKSPACE_RUN_MODE,
  toWorkspaceView
} from "../agent/lib/workspace.ts";

const now = "2026-08-22T12:00:00.000Z";

function workspace(changes = {}) {
  return {
    agent: "eve",
    createdAt: now,
    id: "ws_abc",
    messages: [],
    sandbox: {
      id: "eve-sandbox-abc",
      provider: "vercel",
      status: "running"
    },
    sessionId: "wrun_abc",
    status: "idle",
    title: "Fix caching",
    updatedAt: now,
    version: 2,
    ...changes
  };
}

test("validates Eve workspace records", () => {
  assert.equal(isWorkspaceRecord(workspace()), true);
  assert.equal(isWorkspaceRecord(workspace({ sessionId: undefined })), true);
  assert.equal(isWorkspaceRecord(workspace({ agent: "fx" })), false);
  assert.equal(isWorkspaceRecord(workspace({ harness: "codex" })), true);
  assert.equal(isWorkspaceRecord(workspace({ harness: "unknown" })), false);
  assert.equal(isWorkspaceRecord(workspace({ version: 1 })), false);
});

test("workspace views whitelist fields and omit opaque state", () => {
  const view = toWorkspaceView({
    ...workspace(),
    activeTurnId: "turn_abc",
    unexpected: "private"
  });
  assert.equal("activeTurnId" in view, false);
  assert.equal("unexpected" in view, false);
  assert.equal(view.sessionId, "wrun_abc");
  assert.equal(view.sandbox.id, "eve-sandbox-abc");
  assert.equal(view.model, DEFAULT_WORKSPACE_MODEL);
  assert.equal(view.harness, undefined);
});

test("workspace summaries omit transcripts and sandbox identifiers", () => {
  const summary = toWorkspaceSummary(
    workspace({
      messages: [
        { createdAt: now, id: "msg_abc", role: "user", text: "secret" }
      ]
    })
  );
  assert.deepEqual(Object.keys(summary).sort(), [
    "createdAt",
    "id",
    "status",
    "title",
    "updatedAt"
  ]);
});

test("workspaces use a resumable conversation session", () => {
  assert.equal(WORKSPACE_RUN_MODE, "conversation");
});

test("validates create bodies", () => {
  assert.deepEqual(parseCreateWorkspaceInput({ title: "  Work  " }), {
    harness: DEFAULT_WORKSPACE_HARNESS,
    model: DEFAULT_WORKSPACE_MODEL,
    title: "Work"
  });
  assert.deepEqual(parseCreateWorkspaceInput({ prompt: "  Fix cache  " }), {
    harness: DEFAULT_WORKSPACE_HARNESS,
    model: DEFAULT_WORKSPACE_MODEL,
    prompt: "Fix cache",
    title: "Fix cache"
  });
  assert.deepEqual(
    parseCreateWorkspaceInput({
      harness: "claude-code",
      model: "anthropic/claude-sonnet-5",
      prompt: "Fix cache"
    }),
    {
      harness: "claude-code",
      model: "anthropic/claude-sonnet-5",
      prompt: "Fix cache",
      title: "Fix cache"
    }
  );
  assert.equal(
    parseCreateWorkspaceInput({ harness: "unknown", prompt: "Fix cache" }),
    null
  );
  assert.equal(
    parseCreateWorkspaceInput({ model: "not a model", prompt: "Fix cache" }),
    null
  );
  assert.equal(parseCreateWorkspaceInput({ title: " ", prompt: " " }), null);
});

test("validates workspace harness identifiers", () => {
  for (const harness of [
    "fx",
    "claude-code",
    "codex",
    "cursor",
    "opencode",
    "pi"
  ]) {
    assert.equal(isWorkspaceHarness(harness), true);
  }
  assert.equal(isWorkspaceHarness("unknown"), false);
});

test("validates workspace model identifiers", () => {
  assert.equal(isWorkspaceModel("openai/gpt-5.6-sol"), true);
  assert.equal(isWorkspaceModel("anthropic/claude-sonnet-5"), true);
  assert.equal(isWorkspaceModel("not a model"), false);
});

test("mutation requests use the browser-facing host behind the Eve proxy", () => {
  const request = new Request("http://127.0.0.1:4274/eve/v1/workspaces", {
    method: "POST",
    headers: {
      "content-type": "application/json; charset=utf-8",
      host: "127.0.0.1:4274",
      origin: "https://factory.example",
      "sec-fetch-site": "same-origin",
      "x-forwarded-host": "factory.example",
      "x-operator-action": "create-workspace"
    }
  });
  assert.equal(isWorkspaceMutationRequest(request, "create-workspace"), true);
  assert.equal(isWorkspaceMutationRequest(request, "another-action"), false);
});

test("mutation requests reject malformed origins", () => {
  const request = new Request("http://127.0.0.1:4274/eve/v1/workspaces", {
    method: "POST",
    headers: {
      "content-type": "application/json",
      origin: "not a URL",
      "x-forwarded-host": "factory.example",
      "x-operator-action": "create-workspace"
    }
  });

  assert.equal(isWorkspaceMutationRequest(request, "create-workspace"), false);
});
