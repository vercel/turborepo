import assert from "node:assert/strict";
import test from "node:test";

import {
  createTerminalSession,
  handleTerminalRequest,
  isAllowedSandboxName
} from "../agent/lib/sandbox-terminal.ts";

test("isAllowedSandboxName accepts only ai-sdk-harness prefixed names", () => {
  assert.equal(isAllowedSandboxName("ai-sdk-harness-session-abc"), true);
  assert.equal(isAllowedSandboxName("ai-sdk-harness"), true);
  assert.equal(isAllowedSandboxName("other-sandbox"), false);
  assert.equal(isAllowedSandboxName(""), false);
});

test("createTerminalSession rejects disallowed sandbox names", async () => {
  await assert.rejects(
    createTerminalSession("untrusted-sandbox", async () => ({
      openInteractive: async () => ({ url: "wss://example.com", token: "tok" })
    })),
    /Sandbox name must start with "ai-sdk-harness"/
  );
});

test("createTerminalSession returns a url and token from openInteractive", async () => {
  const session = await createTerminalSession(
    "ai-sdk-harness-session-abc",
    async () => ({
      openInteractive: async () => ({
        url: "wss://example.com/ws",
        token: "secret-token"
      })
    })
  );
  assert.equal(session.url, "wss://example.com/ws");
  assert.equal(session.token, "secret-token");
});

test("createTerminalSession propagates openInteractive errors", async () => {
  await assert.rejects(
    createTerminalSession("ai-sdk-harness-session-abc", async () => ({
      openInteractive: async () => {
        throw new Error("sandbox not found");
      }
    })),
    /sandbox not found/
  );
});

test("handleTerminalRequest returns a session for a valid sandbox", async () => {
  const response = await handleTerminalRequest(
    new Request("http://localhost/api/sandbox/terminal", {
      method: "POST",
      body: JSON.stringify({ sandboxName: "ai-sdk-harness-session-abc" })
    }),
    async () => ({ url: "wss://example.com/ws", token: "tok" })
  );
  assert.equal(response.status, 200);
  const body = await response.json();
  assert.equal(body.url, "wss://example.com/ws");
  assert.equal(body.token, "tok");
});

test("handleTerminalRequest rejects missing sandboxName", async () => {
  const response = await handleTerminalRequest(
    new Request("http://localhost/api/sandbox/terminal", {
      method: "POST",
      body: JSON.stringify({})
    })
  );
  assert.equal(response.status, 400);
  const body = await response.json();
  assert.match(body.error, /A valid sandboxName is required/);
});

test("handleTerminalRequest rejects disallowed sandbox names", async () => {
  const response = await handleTerminalRequest(
    new Request("http://localhost/api/sandbox/terminal", {
      method: "POST",
      body: JSON.stringify({ sandboxName: "untrusted-sandbox" })
    })
  );
  assert.equal(response.status, 400);
  const body = await response.json();
  assert.match(body.error, /Sandbox name is not allowed/);
});

test("handleTerminalRequest returns 404 for not_found errors", async () => {
  const response = await handleTerminalRequest(
    new Request("http://localhost/api/sandbox/terminal", {
      method: "POST",
      body: JSON.stringify({ sandboxName: "ai-sdk-harness-missing" })
    }),
    async () => {
      throw new Error("sandbox not_found");
    }
  );
  assert.equal(response.status, 404);
  const body = await response.json();
  assert.match(body.error, /not_found/);
});

test("handleTerminalRequest returns 500 for unexpected errors", async () => {
  const response = await handleTerminalRequest(
    new Request("http://localhost/api/sandbox/terminal", {
      method: "POST",
      body: JSON.stringify({ sandboxName: "ai-sdk-harness-error" })
    }),
    async () => {
      throw new Error("something went wrong");
    }
  );
  assert.equal(response.status, 500);
  const body = await response.json();
  assert.match(body.error, /something went wrong/);
});
