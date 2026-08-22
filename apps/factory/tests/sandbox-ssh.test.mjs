import assert from "node:assert/strict";
import test from "node:test";

import {
  isSandboxSSHable,
  sandboxSshCommand
} from "../agent/lib/sandbox-ssh.ts";

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

test("isSandboxSSHable recognizes running and stopped sandboxes", () => {
  assert.equal(isSandboxSSHable("running"), true);
  assert.equal(isSandboxSSHable("stopped"), true);
  assert.equal(isSandboxSSHable("provisioning"), false);
  assert.equal(isSandboxSSHable("pending"), false);
  assert.equal(isSandboxSSHable("failed"), false);
  assert.equal(isSandboxSSHable("aborted"), false);
  assert.equal(isSandboxSSHable("snapshotting"), false);
});
