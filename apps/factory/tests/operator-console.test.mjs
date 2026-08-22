import assert from "node:assert/strict";
import test from "node:test";

import {
  isOperatorChatRequest,
  OPERATOR_CHAT_PRINCIPAL
} from "../agent/lib/operator-console.ts";
import { isAppPrincipal } from "../agent/lib/repo.ts";

const CONSOLE = "turborepo-factory.vercel.app";

function request(headers) {
  return { headers: { get: (name) => headers[name] ?? null } };
}

test("accepts a marked same-origin console request", () => {
  assert.equal(
    isOperatorChatRequest(
      request({
        host: CONSOLE,
        origin: `https://${CONSOLE}`,
        "sec-fetch-site": "same-origin",
        "x-operator-action": "open-operator-chat"
      })
    ),
    true
  );
});

test("matches the forwarded host rather than the proxied one", () => {
  assert.equal(
    isOperatorChatRequest(
      request({
        host: "127.0.0.1:4274",
        origin: `https://${CONSOLE}`,
        "x-forwarded-host": CONSOLE,
        "x-operator-action": "open-operator-chat"
      })
    ),
    true
  );
});

test("accepts the stream request browsers send without an origin", () => {
  assert.equal(
    isOperatorChatRequest(
      request({
        host: CONSOLE,
        "sec-fetch-site": "same-origin",
        "x-operator-action": "open-operator-chat"
      })
    ),
    true
  );
});

test("rejects unmarked and cross-site requests", () => {
  assert.equal(isOperatorChatRequest(request({})), false);
  assert.equal(
    isOperatorChatRequest(
      request({ "x-operator-action": "run-daily-performance" })
    ),
    false
  );
  assert.equal(
    isOperatorChatRequest(
      request({
        host: CONSOLE,
        origin: "https://attacker.example",
        "x-operator-action": "open-operator-chat"
      })
    ),
    false
  );
  assert.equal(
    isOperatorChatRequest(
      request({
        host: CONSOLE,
        origin: "null",
        "x-operator-action": "open-operator-chat"
      })
    ),
    false
  );
  assert.equal(
    isOperatorChatRequest(
      request({
        host: CONSOLE,
        origin: `https://${CONSOLE}`,
        "sec-fetch-site": "same-site",
        "x-operator-action": "open-operator-chat"
      })
    ),
    false
  );
  assert.equal(
    isOperatorChatRequest(
      request({
        "sec-fetch-site": "cross-site",
        "x-operator-action": "open-operator-chat"
      })
    ),
    false
  );
});

test("chats as a user, never as the app principal", () => {
  assert.equal(isAppPrincipal(OPERATOR_CHAT_PRINCIPAL), false);
  assert.equal(OPERATOR_CHAT_PRINCIPAL.principalType, "user");
});
