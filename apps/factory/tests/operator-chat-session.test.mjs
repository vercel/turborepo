import assert from "node:assert/strict";
import test from "node:test";

import {
  MAX_SAVED_CHAT_BYTES,
  parseSavedChat,
  serializeSavedChat
} from "../agent/lib/operator-chat-session.ts";

const session = { sessionId: "ses_01JABCDEFG", streamIndex: 12 };

test("round-trips a saved thread", () => {
  const events = [{ data: { text: "hello" }, type: "message.completed" }];
  const saved = parseSavedChat(serializeSavedChat({ events, session }));
  assert.deepEqual(saved, { events, session });
});

test("keeps the cursor when the event log is too large to store", () => {
  const events = [
    {
      data: { text: "x".repeat(MAX_SAVED_CHAT_BYTES) },
      type: "message.completed"
    }
  ];
  const saved = parseSavedChat(serializeSavedChat({ events, session }));
  assert.deepEqual(saved, { events: [], session });
});

test("saves nothing without a session cursor", () => {
  assert.equal(serializeSavedChat({ events: [], session: undefined }), null);
  assert.equal(
    serializeSavedChat({ events: [], session: { sessionId: "ses_1" } }),
    null
  );
});

test("ignores unreadable stored threads", () => {
  assert.equal(parseSavedChat(null), null);
  assert.equal(parseSavedChat("not json"), null);
  assert.equal(parseSavedChat("[]"), null);
  assert.equal(parseSavedChat(JSON.stringify({ events: [] })), null);
  assert.deepEqual(
    parseSavedChat(JSON.stringify({ events: "nope", session })),
    { events: [], session }
  );
});
