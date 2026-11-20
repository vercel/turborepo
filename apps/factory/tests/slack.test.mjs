import assert from "node:assert/strict";
import test from "node:test";

import {
  deliverSlackMessage,
  markPullRequestSlackNotificationMerged
} from "../agent/lib/slack.ts";

const logger = { error() {}, info() {} };

test("reports successful Slack delivery details", async () => {
  const entries = [];
  const result = await deliverSlackMessage("test", {
    channel: "C123",
    event: "test",
    logger: {
      error() {},
      info(...entry) {
        entries.push(entry);
      }
    },
    metadata: { pullRequestNumber: 123 },
    send: async ({ channel, text }) => {
      assert.deepEqual({ channel, text }, { channel: "C123", text: "test" });
      return { ok: true, channel, ts: "123.456" };
    }
  });

  assert.deepEqual(result, {
    ok: true,
    channel: "C123",
    timestamp: "123.456"
  });
  assert.deepEqual(entries, [
    [
      "Slack message delivered.",
      {
        event: "test",
        channel: "C123",
        timestamp: "123.456",
        pullRequestNumber: 123
      }
    ]
  ]);
});

test("passes a thread timestamp to Slack replies", async () => {
  const result = await deliverSlackMessage("reason", {
    channel: "C123",
    event: "test",
    logger,
    threadTimestamp: "123.456",
    send: async (input) => {
      assert.deepEqual(input, {
        channel: "C123",
        text: "reason",
        threadTimestamp: "123.456"
      });
      return { ok: true, channel: "C123", ts: "124.000" };
    }
  });

  assert.equal(result.ok, true);
});

test("uses configured fallback values for incomplete success responses", async () => {
  const result = await deliverSlackMessage("test", {
    channel: "C123",
    event: "test",
    logger,
    send: async () => ({ ok: true })
  });

  assert.deepEqual(result, {
    ok: true,
    channel: "C123",
    timestamp: null
  });
});

test("reports Slack API errors", async () => {
  const result = await deliverSlackMessage("test", {
    channel: "C123",
    event: "test",
    logger,
    send: async () => ({ ok: false, error: "channel_not_found" })
  });

  assert.deepEqual(result, {
    ok: false,
    channel: "C123",
    error: "Slack API returned channel_not_found."
  });
});

test("reports Slack API rejection without exposing the response", async () => {
  const result = await deliverSlackMessage("test", {
    channel: "C123",
    event: "test",
    logger,
    send: async () => ({ ok: false })
  });

  assert.deepEqual(result, {
    ok: false,
    channel: "C123",
    error: "Slack API rejected the message."
  });
});

test("reports credential and transport errors", async () => {
  const result = await deliverSlackMessage("test", {
    channel: "C123",
    event: "test",
    logger,
    send: async () => {
      throw new Error("Slack credentials are unavailable.");
    }
  });

  assert.deepEqual(result, {
    ok: false,
    channel: "C123",
    error: "Slack credentials are unavailable."
  });
});

test("sanitizes unexpected Slack transport errors", async () => {
  const result = await deliverSlackMessage("test", {
    channel: "C123",
    event: "test",
    logger,
    send: async () => {
      throw new Error("request failed with sensitive implementation details");
    }
  });

  assert.deepEqual(result, {
    ok: false,
    channel: "C123",
    error: "Slack delivery failed before the API accepted the message."
  });
});

test("reports Slack delivery timeouts", async () => {
  const result = await deliverSlackMessage("test", {
    channel: "C123",
    event: "test",
    logger,
    send: () => new Promise(() => {}),
    timeoutMs: 1
  });

  assert.deepEqual(result, {
    ok: false,
    channel: "C123",
    error: "Slack delivery status is unknown because the request timed out."
  });
});

test("ignores diagnostic logger failures", async () => {
  const result = await deliverSlackMessage("test", {
    channel: "C123",
    event: "test",
    logger: {
      error() {
        throw new Error("logger failed");
      },
      info() {
        throw new Error("logger failed");
      }
    },
    send: async () => ({ ok: true })
  });

  assert.deepEqual(result, {
    ok: true,
    channel: "C123",
    timestamp: null
  });
});

test("updates the recorded pull request notification after merge", async () => {
  const requests = [];
  const updated = await markPullRequestSlackNotificationMerged(
    123,
    ":pr-merged: merged",
    {
      notification: { channel: "C123", timestamp: "123.456" },
      request: async (operation, body) => {
        requests.push({ operation, body });
        return { ok: true };
      }
    }
  );

  assert.equal(updated, true);
  assert.deepEqual(requests, [
    {
      operation: "chat.update",
      body: {
        channel: "C123",
        ts: "123.456",
        text: ":pr-merged: merged"
      }
    }
  ]);
});

test("does not post a new message when no pull request notification was recorded", async () => {
  const operations = [];
  const updated = await markPullRequestSlackNotificationMerged(123, "merged", {
    notification: null,
    request: async (operation) => {
      operations.push(operation);
      return { ok: true };
    }
  });
  assert.equal(updated, false);
  assert.deepEqual(operations, []);
});
