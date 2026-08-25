import assert from "node:assert/strict";
import test from "node:test";

import {
  isOperatorSessionRequest,
  operatorSessionPrincipal,
  OPERATOR_SESSION_ACTION,
  OPERATOR_SESSION_PRINCIPAL,
  selectedOperatorModel
} from "../agent/lib/operator-console.ts";
import { isAppPrincipal } from "../agent/lib/repo.ts";

const CONSOLE = "turborepo-factory.vercel.app";

function request(headers) {
  return { headers: { get: (name) => headers[name] ?? null } };
}

function sessionRequest(headers = {}) {
  return request({
    "x-operator-action": OPERATOR_SESSION_ACTION,
    ...headers
  });
}

test("accepts a marked same-origin workspace session request", () => {
  assert.equal(
    isOperatorSessionRequest(
      sessionRequest({
        host: CONSOLE,
        origin: `https://${CONSOLE}`,
        "sec-fetch-site": "same-origin"
      })
    ),
    true
  );
});

test("matches the forwarded host rather than the proxied one", () => {
  assert.equal(
    isOperatorSessionRequest(
      sessionRequest({
        host: "127.0.0.1:4274",
        origin: `https://${CONSOLE}`,
        "x-forwarded-host": CONSOLE
      })
    ),
    true
  );
});

test("accepts the stream request browsers send without an origin", () => {
  assert.equal(
    isOperatorSessionRequest(
      sessionRequest({
        host: CONSOLE,
        "sec-fetch-site": "same-origin"
      })
    ),
    true
  );
});

test("rejects unmarked and cross-site requests", () => {
  assert.equal(isOperatorSessionRequest(request({})), false);
  assert.equal(
    isOperatorSessionRequest(
      request({ "x-operator-action": "run-daily-performance" })
    ),
    false
  );
  assert.equal(
    isOperatorSessionRequest(
      sessionRequest({ host: CONSOLE, origin: "https://attacker.example" })
    ),
    false
  );
  assert.equal(
    isOperatorSessionRequest(sessionRequest({ host: CONSOLE, origin: "null" })),
    false
  );
  assert.equal(
    isOperatorSessionRequest(
      sessionRequest({
        host: CONSOLE,
        origin: `https://${CONSOLE}`,
        "sec-fetch-site": "same-site"
      })
    ),
    false
  );
  assert.equal(
    isOperatorSessionRequest(
      sessionRequest({ "sec-fetch-site": "cross-site" })
    ),
    false
  );
});

test("workspace sessions run as a user, never as the app principal", () => {
  assert.equal(isAppPrincipal(OPERATOR_SESSION_PRINCIPAL), false);
  assert.equal(OPERATOR_SESSION_PRINCIPAL.principalType, "user");
});

test("workspace principals carry their selected model", () => {
  const principal = operatorSessionPrincipal("openai/gpt-5.6-sol");
  assert.equal(selectedOperatorModel(principal), "openai/gpt-5.6-sol");
  assert.equal(isAppPrincipal(principal), false);
});

test("workspace principals ignore malformed model identifiers", () => {
  const principal = operatorSessionPrincipal("not a model");
  assert.equal(principal, OPERATOR_SESSION_PRINCIPAL);
  assert.equal(selectedOperatorModel(principal), undefined);
});
