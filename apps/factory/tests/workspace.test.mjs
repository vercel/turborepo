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
  reconcileWorkspacePullRequest,
  recordWorkspaceWorkflowRun,
  toWorkspaceSummary,
  toWorkspaceView,
  workspaceActionActivity,
  workspaceSandboxName,
  workspaceStatusAfterTurn
} from "../agent/lib/workspace.ts";

const now = "2026-08-22T12:00:00.000Z";

function workspace(changes = {}) {
  return {
    agent: "fx",
    createdAt: now,
    id: "ws_abc",
    messages: [],
    publishToken: "private-publish-token",
    sandbox: {
      name: workspaceSandboxName("ws_abc"),
      provider: "vercel",
      status: "running"
    },
    sessionId: "1770000000000-1770000000000000000-a1b2c3d4e5f60718",
    status: "idle",
    title: "Fix caching",
    updatedAt: now,
    version: 1,
    ...changes
  };
}

test("validates durable workspace records and deterministic sandbox names", () => {
  assert.equal(isWorkspaceRecord(workspace()), true);
  assert.equal(isWorkspaceRecord(workspace({ sessionId: undefined })), true);
  assert.equal(
    isWorkspaceRecord(
      workspace({ sandbox: { name: "another-sandbox", provider: "vercel" } })
    ),
    false
  );
  assert.equal(isWorkspaceRecord(workspace({ agent: "opencode" })), false);
});

test("workspace views whitelist fields and omit opaque state", () => {
  const view = toWorkspaceView({ ...workspace(), unexpected: "private" });
  assert.equal("activeTurnId" in view, false);
  assert.equal("activeDispatchId" in view, false);
  assert.equal("publishToken" in view, false);
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

test("workspace views expose the exact fx resume command", () => {
  const view = toWorkspaceView(workspace());
  assert.equal(
    view.chatCommand,
    "fx resume --id 1770000000000-1770000000000000000-a1b2c3d4e5f60718"
  );
  assert.equal(
    toWorkspaceView(workspace({ sessionId: undefined })).chatCommand,
    undefined
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


test("workspace pull request state drives completion", () => {
  const withPullRequest = workspace({
    pullRequest: {
      number: 123,
      url: "https://github.com/vercel/turborepo/pull/123"
    }
  });
  const merged = reconcileWorkspacePullRequest(
    withPullRequest,
    123,
    "merged",
    now
  );
  assert.equal(merged.status, "done");
  assert.equal(merged.pullRequest.state, "merged");
  assert.equal(workspaceStatusAfterTurn(merged), "idle");
  assert.equal(
    reconcileWorkspacePullRequest(merged, 123, "open", now),
    merged
  );
  assert.equal(
    reconcileWorkspacePullRequest(
      {
        ...withPullRequest,
        pullRequest: {
          ...withPullRequest.pullRequest,
          checkedAt: "2026-08-22T12:01:00.000Z",
          state: "open"
        }
      },
      123,
      "closed",
      now
    ).pullRequest.state,
    "open"
  );
  assert.equal(
    reconcileWorkspacePullRequest(withPullRequest, 456, "merged", now),
    withPullRequest
  );

  const running = reconcileWorkspacePullRequest(
    { ...withPullRequest, activity: "Waiting for input", status: "running" },
    123,
    "merged",
    now
  );
  assert.equal(running.status, "running");
  assert.equal(running.activity, "Waiting for input");
  assert.equal(running.completeAfterTurn, true);
  assert.equal(workspaceStatusAfterTurn(running), "done");
  assert.equal(
    reconcileWorkspacePullRequest(
      { ...merged, status: "running" },
      123,
      "merged",
      "2026-08-22T12:02:00.000Z"
    ).completeAfterTurn,
    undefined
  );

  const continued = beginWorkspaceTurn(merged, {
    createdAt: "2026-08-22T12:01:00.000Z",
    id: "turn_after_merge",
    text: "Make one more change"
  });
  assert.equal(continued.completeAfterTurn, undefined);
  assert.equal(workspaceStatusAfterTurn(continued), "idle");
});

test("Eve action names become useful workspace activity", () => {
  assert.equal(
    workspaceActionActivity([
      { kind: "tool-call", toolName: "run_example_turbo_tasks" }
    ]),
    "Running run example turbo tasks"
  );
  assert.equal(
    workspaceActionActivity([{ kind: "subagent-call", name: "reviewer" }]),
    "Delegating to reviewer"
  );
  assert.equal(
    workspaceActionActivity([
      { kind: "tool-call", toolName: "read_file" },
      { kind: "tool-call", toolName: "bash" }
    ]),
    "Running 2 actions"
  );
});

test("validates create and turn bodies", () => {
  assert.deepEqual(parseCreateWorkspaceInput({ title: "  Work  " }), {
    title: "Work"
  });
  assert.deepEqual(
    parseCreateWorkspaceInput({
      prompt: "  Fix cache  "
    }),
    {
      prompt: "Fix cache",
      title: "Fix cache"
    }
  );
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
