import assert from "node:assert/strict";
import test from "node:test";

import {
  isWorkspaceMutationRequest,
  isWorkspaceRecord,
  parseCreateWorkspaceInput,
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
    title: "Work"
  });
  assert.deepEqual(
    parseCreateWorkspaceInput({ prompt: "  Fix cache  " }),
    { prompt: "Fix cache", title: "Fix cache" }
  );
  assert.equal(parseCreateWorkspaceInput({ title: " ", prompt: " " }), null);
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
