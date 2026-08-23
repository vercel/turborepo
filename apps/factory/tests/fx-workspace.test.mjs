import assert from "node:assert/strict";
import test from "node:test";

import { workspaceNetworkPolicy } from "../agent/lib/workspace-network-policy.ts";

test("workspace GitHub credential rules precede the catch-all rule", () => {
  const policy = workspaceNetworkPolicy("github-token");
  assert.notEqual(typeof policy, "string");
  assert.ok(policy.allow && !Array.isArray(policy.allow));

  assert.deepEqual(Object.keys(policy.allow), [
    "api.github.com",
    "github.com",
    "*"
  ]);
  assert.deepEqual(policy.allow["github.com"][0].transform, [
    {
      headers: {
        authorization: `Basic ${Buffer.from("x-access-token:github-token").toString("base64")}`
      }
    }
  ]);
});
