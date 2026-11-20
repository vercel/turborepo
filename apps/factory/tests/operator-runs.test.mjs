import assert from "node:assert/strict";
import test from "node:test";

import {
  createOperatorWorkspaceRecord,
  isOperatorRunRequest,
  MAINTENANCE_RUN_ACTION
} from "../agent/lib/operator-runs.ts";

function request(headers) {
  return { headers: new Headers(headers) };
}

test("operator runs create workspace records for the home page", () => {
  const workspace = createOperatorWorkspaceRecord({
    id: "ws_operator",
    now: "2026-08-24T20:00:00.000Z",
    prompt: "Improve the selected example.",
    title: "Daily example maintenance · with-vue-nuxt",
    turnId: "turn_operator"
  });

  assert.equal(workspace.id, "ws_operator");
  assert.equal(workspace.status, "running");
  assert.equal(workspace.messages[0]?.text, "Improve the selected example.");
  assert.equal(workspace.title, "Daily example maintenance · with-vue-nuxt");
});

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
