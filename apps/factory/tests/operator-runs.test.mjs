import assert from "node:assert/strict";
import test from "node:test";

import {
  isOperatorRunRequest,
  MAINTENANCE_RUN_ACTION
} from "../agent/lib/operator-runs.ts";

function request(headers) {
  return { headers: new Headers(headers) };
}

test("operator runs match the browser-facing host behind the Eve proxy", () => {
  assert.equal(
    isOperatorRunRequest(
      request({
        "content-type": "application/json",
        host: "127.0.0.1:8080",
        origin: "http://localhost:3000",
        "sec-fetch-site": "same-origin",
        "x-forwarded-host": "localhost:3000",
        "x-operator-action": MAINTENANCE_RUN_ACTION
      }),
      MAINTENANCE_RUN_ACTION
    ),
    true
  );
});

test("operator runs reject cross-site and incorrectly marked requests", () => {
  for (const headers of [
    {
      "content-type": "application/json",
      host: "factory.example.com",
      origin: "https://evil.example.com",
      "x-operator-action": MAINTENANCE_RUN_ACTION
    },
    {
      "content-type": "application/json",
      host: "factory.example.com",
      origin: "https://factory.example.com",
      "sec-fetch-site": "cross-site",
      "x-operator-action": MAINTENANCE_RUN_ACTION
    },
    {
      "content-type": "application/json",
      host: "factory.example.com",
      origin: "https://factory.example.com",
      "x-operator-action": "wrong-action"
    }
  ]) {
    assert.equal(
      isOperatorRunRequest(request(headers), MAINTENANCE_RUN_ACTION),
      false
    );
  }
});
