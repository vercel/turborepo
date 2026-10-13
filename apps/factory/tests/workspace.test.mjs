import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import {
  isWorkspaceMutationRequest,
  isWorkspaceModel,
  isWorkspaceRecord,
  isWorkspaceThinkingEffort,
  parseCreateWorkspaceInput,
  DEFAULT_WORKSPACE_THINKING_EFFORT,
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

test("validates Eve workspace records, including legacy coding-agent metadata", () => {
  assert.equal(isWorkspaceRecord(workspace()), true);
  assert.equal(isWorkspaceRecord(workspace({ sessionId: undefined })), true);
  assert.equal(isWorkspaceRecord(workspace({ agent: "fx" })), false);
  assert.equal(isWorkspaceRecord(workspace({ harness: "codex" })), true);
  assert.equal(
    isWorkspaceRecord(workspace({ thinkingEffort: "medium" })),
    true
  );
  assert.equal(isWorkspaceRecord(workspace({ thinkingEffort: "max" })), false);
  assert.equal(isWorkspaceRecord(workspace({ version: 1 })), false);
});

test("workspace views whitelist fields and omit opaque state", () => {
  const view = toWorkspaceView({
    ...workspace({ harness: "codex" }),
    activeTurnId: "turn_abc",
    unexpected: "private"
  });
  assert.equal("activeTurnId" in view, false);
  assert.equal("unexpected" in view, false);
  assert.equal(view.sessionId, "wrun_abc");
  assert.equal(view.sandbox.id, "eve-sandbox-abc");
  assert.equal(view.model, DEFAULT_WORKSPACE_MODEL);
  assert.equal("harness" in view, false);
  assert.equal(view.thinkingEffort, DEFAULT_WORKSPACE_THINKING_EFFORT);
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
    model: DEFAULT_WORKSPACE_MODEL,
    thinkingEffort: DEFAULT_WORKSPACE_THINKING_EFFORT,
    title: "Work"
  });
  assert.deepEqual(parseCreateWorkspaceInput({ prompt: "  Fix cache  " }), {
    model: DEFAULT_WORKSPACE_MODEL,
    prompt: "Fix cache",
    thinkingEffort: DEFAULT_WORKSPACE_THINKING_EFFORT,
    title: "Fix cache"
  });
  assert.deepEqual(
    parseCreateWorkspaceInput({
      model: "anthropic/claude-sonnet-5",
      prompt: "Fix cache",
      thinkingEffort: "medium"
    }),
    {
      model: "anthropic/claude-sonnet-5",
      prompt: "Fix cache",
      thinkingEffort: "medium",
      title: "Fix cache"
    }
  );
  assert.equal(
    parseCreateWorkspaceInput({ model: "not a model", prompt: "Fix cache" }),
    null
  );
  assert.equal(
    parseCreateWorkspaceInput({ prompt: "Fix cache", thinkingEffort: "max" }),
    null
  );
  assert.equal(parseCreateWorkspaceInput({ title: " ", prompt: " " }), null);
});

test("validates workspace thinking effort", () => {
  for (const effort of ["low", "medium", "high"]) {
    assert.equal(isWorkspaceThinkingEffort(effort), true);
  }
  assert.equal(isWorkspaceThinkingEffort("max"), false);
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

test("new workspaces default to Sol 6.1 and preserve explicit Fast selections", () => {
  assert.equal(DEFAULT_WORKSPACE_MODEL, "openai/gpt-6.1-sol");
  assert.equal(
    parseCreateWorkspaceInput({ prompt: "Fix cache" }).model,
    "openai/gpt-6.1-sol"
  );
  assert.equal(
    parseCreateWorkspaceInput({
      prompt: "Fix cache",
      model: "openai/gpt-6.1-sol-fast"
    }).model,
    "openai/gpt-6.1-sol-fast"
  );
});

test("Start work puts the workspace composer before scheduled jobs", () => {
  const page = readFileSync(
    new URL("../app/work/page.tsx", import.meta.url),
    "utf8"
  );
  const composer = page.indexOf("<WorkspaceComposer ");
  const schedules = page.indexOf('aria-labelledby="manual-schedules-title"');
  assert.ok(composer !== -1 && schedules > composer);
});

test("mobile form text prevents focus zoom without restricting browser zoom", () => {
  const css = readFileSync(
    new URL("../app/globals.css", import.meta.url),
    "utf8"
  );
  assert.match(
    css,
    /@media \(max-width: 720px\), \(pointer: coarse\)\s*\{\s*input,\s*select,\s*textarea\s*\{\s*font-size: max\(16px, 1rem\);/
  );
  const layout = readFileSync(
    new URL("../app/layout.tsx", import.meta.url),
    "utf8"
  );
  assert.doesNotMatch(layout, /userScalable|maximumScale/);
});

test("legacy coding-agent selections are ignored for new workspaces", () => {
  const input = parseCreateWorkspaceInput({
    prompt: "Fix cache",
    harness: "codex"
  });
  assert.equal("harness" in input, false);
  assert.equal(input.model, DEFAULT_WORKSPACE_MODEL);
});

test("workspace coding uses Eve's direct tools without a runtime picker", () => {
  const instructions = readFileSync(
    new URL("../agent/instructions.md", import.meta.url),
    "utf8"
  );
  assert.match(
    instructions,
    /run validation directly with `bash`, `read_file`, and `write_file`/
  );
  assert.doesNotMatch(instructions, /harness_agent/);
  const composer = readFileSync(
    new URL("../app/workspace-composer.tsx", import.meta.url),
    "utf8"
  );
  assert.doesNotMatch(composer, /workspace-harness|Coding agent/);
  const manifest = JSON.parse(
    readFileSync(new URL("../package.json", import.meta.url), "utf8")
  );
  assert.equal(
    Object.keys(manifest.dependencies).some(
      (name) =>
        name.startsWith("@ai-sdk/harness") || name === "@ai-sdk/sandbox-vercel"
    ),
    false
  );
  assert.equal(manifest.scripts.build, "eve build && next build");
});
