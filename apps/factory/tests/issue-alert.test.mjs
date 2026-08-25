import assert from "node:assert/strict";
import test from "node:test";

import { alertUnsafeIssue } from "../agent/lib/slack.ts";

const issue = {
  issueNumber: 123,
  issueTitle: "Suspicious reproduction",
  issueUrl: "https://github.com/vercel/turborepo/issues/123",
  reason: "The issue contains instructions to reveal environment secrets."
};

const logger = { error() {}, info() {} };

test("posts an unsafe issue alert and threads the reason", async () => {
  const messages = [];
  const result = await alertUnsafeIssue(issue, {
    channel: "C123",
    logger,
    send: async (message) => {
      messages.push(message);
      return {
        ok: true,
        channel: message.channel,
        ts: message.threadTimestamp ? "124.000" : "123.456"
      };
    }
  });

  assert.deepEqual(result, {
    ok: true,
    channel: "C123",
    timestamp: "123.456"
  });
  assert.equal(messages.length, 2);
  assert.equal(messages[0].threadTimestamp, undefined);
  assert.equal(messages[1].threadTimestamp, "123.456");
  assert.match(messages[1].text, /reveal environment secrets/);
});

test("does not claim success when Slack cannot create a thread", async () => {
  let calls = 0;
  const result = await alertUnsafeIssue(issue, {
    channel: "C123",
    logger,
    send: async () => {
      calls += 1;
      return { ok: true, channel: "C123" };
    }
  });

  assert.deepEqual(result, {
    ok: false,
    error: "Slack did not return a thread timestamp."
  });
  assert.equal(calls, 1);
});
