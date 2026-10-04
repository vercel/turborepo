import assert from "node:assert/strict";
import test from "node:test";

import {
  isWorkspaceRunning,
  latestWorkspaceFailure,
  removeConfirmedQueuedMessages,
  workspaceStatusLabel
} from "../app/workspace-types.ts";

test("workspace status labels describe operator-visible state", () => {
  assert.equal(workspaceStatusLabel("idle"), "Ready");
  assert.equal(workspaceStatusLabel("running"), "Working");
  assert.equal(workspaceStatusLabel("pending"), "Working");
  assert.equal(workspaceStatusLabel("error"), "Error");
  assert.equal(isWorkspaceRunning("running"), true);
  assert.equal(isWorkspaceRunning("idle"), false);
});

test("projects actionable failure information from workspace events", () => {
  const events = [
    { type: "turn.started", data: { turnId: "turn_1" } },
    {
      type: "step.failed",
      data: {
        code: "gateway-auth",
        message: "Failed",
        details: {
          hint: "Refresh the AI Gateway credentials.",
          detail: "401 Unauthorized\nCaused by an expired token."
        }
      }
    },
    {
      type: "turn.failed",
      data: {
        code: "gateway-auth",
        message: "Failed",
        details: {
          hint: "Refresh the AI Gateway credentials.",
          detail: "401 Unauthorized\nCaused by an expired token."
        }
      }
    }
  ];
  assert.deepEqual(latestWorkspaceFailure(events), {
    code: "gateway-auth",
    message: "Failed",
    hint: "Refresh the AI Gateway credentials.",
    detail: "401 Unauthorized\nCaused by an expired token."
  });
});

test("clears an old workspace failure when a later turn starts", () => {
  assert.equal(
    latestWorkspaceFailure([
      {
        type: "session.failed",
        data: { code: "failed", message: "First run failed" }
      },
      { type: "turn.started", data: { turnId: "turn_2" } }
    ]),
    undefined
  );
});

test("ignores malformed workspace failure events", () => {
  assert.equal(
    latestWorkspaceFailure([
      { type: "turn.failed", data: { code: "failed", message: "  " } }
    ]),
    undefined
  );
});

function userMessage(text) {
  return { role: "user", parts: [{ type: "text", text }] };
}

test("queue cleanup preserves state identity when there is nothing to confirm", () => {
  const empty = [];
  assert.equal(removeConfirmedQueuedMessages(empty, []), empty);
  const queue = [{ id: "queued_1", text: "Fix cache", afterMessageCount: 1 }];
  // A repeated older prompt and an assistant response do not acknowledge a queue entry.
  const messages = [
    userMessage("Fix cache"),
    { role: "assistant", parts: [{ type: "text", text: "Fix cache" }] }
  ];
  assert.equal(removeConfirmedQueuedMessages(queue, messages), queue);
  // Even freshly rebuilt projections must not trigger another state update.
  for (let render = 0; render < 100; render++) {
    assert.equal(
      removeConfirmedQueuedMessages(queue, structuredClone(messages)),
      queue
    );
  }
});

test("queue cleanup removes only confirmed messages and then stabilizes", () => {
  const queue = [
    { id: "queued_1", text: "Fix cache", afterMessageCount: 1 },
    { id: "queued_2", text: "Run tests", afterMessageCount: 1 }
  ];
  const messages = [
    userMessage("Original request"),
    userMessage("  Fix cache  ")
  ];
  const pending = removeConfirmedQueuedMessages(queue, messages);
  assert.deepEqual(pending, [queue[1]]);
  assert.notEqual(pending, queue);
  assert.equal(pending[0], queue[1]);
  assert.equal(queue.length, 2);
  assert.equal(
    removeConfirmedQueuedMessages(pending, structuredClone(messages)),
    pending
  );
  const empty = removeConfirmedQueuedMessages(pending, [
    ...messages,
    userMessage("Run tests")
  ]);
  assert.deepEqual(empty, []);
  assert.equal(
    removeConfirmedQueuedMessages(empty, structuredClone(messages)),
    empty
  );
});

test("queue confirmation combines text parts and ignores non-text parts", () => {
  const queue = [{ text: "Fix\ncache", afterMessageCount: 0 }];
  const messages = [
    {
      role: "user",
      parts: [
        { type: "text", text: "Fix" },
        { type: "file" },
        { type: "text", text: "cache" }
      ]
    }
  ];
  assert.deepEqual(removeConfirmedQueuedMessages(queue, messages), []);
});
