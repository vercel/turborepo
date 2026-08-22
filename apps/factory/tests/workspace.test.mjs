import assert from "node:assert/strict";
import test from "node:test";

import {
  beginWorkspaceTurn,
  isSafeWorkspaceDiffPath,
  isWorkspaceMutationRequest,
  isWorkspaceRecord,
  parseCreateWorkspaceInput,
  parseWorkspaceTurnInput,
  recoverTerminalWorkspaceTurn,
  recordWorkspaceWorkflowRun,
  toWorkspaceSummary,
  toWorkspaceView,
  workspaceSandboxName
} from "../agent/lib/workspace.ts";

const now = "2026-08-22T12:00:00.000Z";

function workspace(changes = {}) {
  const sessionId = "ses_factory_abc";
  return {
    createdAt: now,
    harness: "opencode",
    id: "ws_abc",
    messages: [],
    resumeState: { secret: "opaque" },
    sandbox: {
      name: workspaceSandboxName(sessionId),
      provider: "vercel",
      status: "running"
    },
    sessionId,
    status: "idle",
    title: "Fix caching",
    updatedAt: now,
    version: 1,
    ...changes
  };
}

test("validates durable workspace records and deterministic sandbox names", () => {
  assert.equal(isWorkspaceRecord(workspace()), true);
  assert.equal(
    isWorkspaceRecord(
      workspace({ sandbox: { name: "another-sandbox", provider: "vercel" } })
    ),
    false
  );
  assert.equal(
    isWorkspaceRecord(workspace({ resumeState: { value: NaN } })),
    false
  );
});

test("workspace views whitelist fields and omit opaque state", () => {
  const view = toWorkspaceView({ ...workspace(), unexpected: "private" });
  assert.equal("resumeState" in view, false);
  assert.equal("activeTurnId" in view, false);
  assert.equal("activeDispatchId" in view, false);
  assert.equal("unexpected" in view, false);
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

test("workspace views expose the command for resuming the OpenCode chat", () => {
  const view = toWorkspaceView(
    workspace({
      resumeState: {
        data: { openCodeSessionId: "ses_opencode_abc" },
        harnessId: "opencode",
        specificationVersion: "harness-v1",
        type: "resume-session"
      }
    })
  );
  assert.equal(
    view.chatCommand,
    "/vercel/sandbox/.harness-bootstrap/opencode/node_modules/.bin/opencode --session ses_opencode_abc"
  );
});

test("workspace diffs omit common untracked credential files", () => {
  assert.equal(isSafeWorkspaceDiffPath("src/index.ts"), true);
  assert.equal(isSafeWorkspaceDiffPath(".env.local"), false);
  assert.equal(isSafeWorkspaceDiffPath("config/signing.pem"), false);
  assert.equal(isSafeWorkspaceDiffPath("nested/.npmrc"), false);
});

test("beginWorkspaceTurn atomically rejects overlap", () => {
  const started = beginWorkspaceTurn(workspace(), {
    createdAt: now,
    id: "turn_abc",
    text: "Please fix it"
  });
  assert.equal(started.status, "running");
  assert.equal(started.messages.at(-1).text, "Please fix it");
  assert.equal(
    beginWorkspaceTurn(started, {
      createdAt: now,
      id: "turn_def",
      text: "Overlap"
    }),
    null
  );
});

test("beginWorkspaceTurn recovers a stale unstarted claim", () => {
  const startedAt = "2026-08-22T10:00:00.000Z";
  const recovered = beginWorkspaceTurn(
    workspace({
      activeTurnId: "turn_stale",
      status: "running",
      updatedAt: startedAt
    }),
    {
      createdAt: "2026-08-22T11:00:00.000Z",
      id: "turn_new",
      text: "Try again"
    }
  );
  assert.equal(recovered.activeTurnId, "turn_new");
  assert.equal(recovered.messages.at(-1).text, "Try again");
});

test("workflow ids attach only to their own active or completed turn", () => {
  const active = workspace({ activeTurnId: "turn_abc", status: "running" });
  assert.equal(
    recordWorkspaceWorkflowRun(active, "turn_abc", "wrun_abc").workflowRunId,
    "wrun_abc"
  );
  const completed = workspace({
    messages: [
      { createdAt: now, id: "msg_turn_abc", role: "assistant", text: "done" }
    ]
  });
  assert.equal(
    recordWorkspaceWorkflowRun(completed, "turn_abc", "wrun_abc").workflowRunId,
    "wrun_abc"
  );
  assert.equal(
    recordWorkspaceWorkflowRun(
      { ...active, activeTurnId: "turn_new" },
      "turn_abc",
      "wrun_old"
    ).workflowRunId,
    undefined
  );
});

test("terminal workflows return a stranded workspace to the operator", () => {
  const running = workspace({
    activeDispatchId: "dispatch_abc",
    activeTurnId: "turn_abc",
    status: "running",
    workflowRunId: "wrun_abc"
  });
  const recovered = recoverTerminalWorkspaceTurn(
    running,
    "wrun_abc",
    "failed",
    now
  );
  assert.equal(recovered.status, "error");
  assert.equal(recovered.activeTurnId, undefined);
  assert.equal(recovered.workflowRunId, "wrun_abc");
  assert.equal(
    recoverTerminalWorkspaceTurn(running, "wrun_abc", "running", now),
    running
  );
  assert.equal(
    recoverTerminalWorkspaceTurn(running, "wrun_new", "failed", now),
    running
  );
});

test("validates create and turn bodies", () => {
  assert.deepEqual(parseCreateWorkspaceInput({ title: "  Work  " }), {
    title: "Work"
  });
  assert.deepEqual(parseCreateWorkspaceInput({ prompt: "  Fix cache  " }), {
    prompt: "Fix cache",
    title: "Fix cache"
  });
  assert.equal(parseCreateWorkspaceInput({ title: " ", prompt: " " }), null);
  assert.deepEqual(parseWorkspaceTurnInput({ message: "  Go  " }), {
    message: "Go"
  });
  assert.equal(parseWorkspaceTurnInput({ message: "x".repeat(20_001) }), null);
});

test("mutation requests require same origin, JSON, and the exact action", () => {
  const request = new Request("https://factory.example/api/workspaces", {
    method: "POST",
    headers: {
      "content-type": "application/json; charset=utf-8",
      origin: "https://factory.example",
      "x-operator-action": "create-workspace"
    }
  });
  assert.equal(isWorkspaceMutationRequest(request, "create-workspace"), true);
  assert.equal(isWorkspaceMutationRequest(request, "another-action"), false);
});
