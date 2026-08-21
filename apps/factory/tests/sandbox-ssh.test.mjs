import assert from "node:assert/strict";
import test from "node:test";

import { sandboxSshCommand } from "../agent/lib/sandbox-ssh.ts";

test("produces a sandbox ssh command for a given sandbox name", () => {
  assert.equal(sandboxSshCommand("my-sandbox"), "sandbox ssh my-sandbox");
  assert.equal(
    sandboxSshCommand("ai-sdk-harness-session-abc123"),
    "sandbox ssh ai-sdk-harness-session-abc123"
  );
});

test("preserves special characters in sandbox names", () => {
  assert.equal(sandboxSshCommand("a-b_c.d"), "sandbox ssh a-b_c.d");
});
