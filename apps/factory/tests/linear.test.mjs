import assert from "node:assert/strict";
import test from "node:test";

import {
  createLinearCredentials,
  linearCredentials,
  linearMcpAuth
} from "../agent/lib/linear.ts";

function withEnvironment(name, value, run) {
  const previous = process.env[name];
  if (value === undefined) delete process.env[name];
  else process.env[name] = value;
  return Promise.resolve().then(run).finally(() => {
    if (previous === undefined) delete process.env[name];
    else process.env[name] = previous;
  });
}

const request = new Request("https://factory.example/eve/v1/linear");

test("Linear credentials resolve lazily and refresh on each use", async () => {
  let resolutions = 0;
  const credentials = createLinearCredentials(() => ({
    accessToken: async () => `token-${++resolutions}`
  }));
  assert.equal(resolutions, 0);
  assert.equal(await credentials.accessToken(), "token-1");
  assert.equal(await credentials.accessToken(), "token-2");
});

test("Linear credentials accept a resolved token", async () => {
  const credentials = createLinearCredentials(() => ({ accessToken: "token" }));
  assert.equal(await credentials.accessToken(), "token");
});

test("Linear credential failures do not expose provider errors", async () => {
  for (const resolve of [
    () => { throw new Error("sensitive provider detail"); },
    () => ({}),
    () => ({ accessToken: async () => { throw new Error("sensitive token detail"); } })
  ]) {
    await assert.rejects(
      createLinearCredentials(resolve).accessToken(),
      { message: "Linear credentials are unavailable." }
    );
  }
});

test("Linear webhook verification delegates the exact request and raw body", async () => {
  const principal = { principalType: "app", principalId: "connect" };
  const credentials = createLinearCredentials(() => ({
    webhookVerifier: (receivedRequest, body) => {
      assert.equal(receivedRequest, request);
      assert.equal(body, '{"event":"AgentSessionEvent"}');
      return principal;
    }
  }));
  assert.equal(
    await credentials.webhookVerifier(request, '{"event":"AgentSessionEvent"}'),
    principal
  );
});

test("Linear webhook verification fails closed", async (t) => {
  const warnings = [];
  t.mock.method(console, "warn", (message) => warnings.push(message));
  for (const resolve of [
    () => ({}),
    () => ({ webhookVerifier: () => null }),
    () => { throw new Error("sensitive configuration detail"); },
    () => ({ webhookVerifier: async () => { throw new Error("sensitive verifier detail"); } })
  ]) {
    assert.equal(
      await createLinearCredentials(resolve).webhookVerifier(request, "body"),
      null
    );
  }
  assert.deepEqual(warnings, [
    "Linear webhook verification failed.",
    "Linear webhook verification failed."
  ]);
});

test("missing Linear configuration only fails when credentials are used", async (t) => {
  t.mock.method(console, "warn", () => {});
  await withEnvironment("LINEAR_CONNECT_UID", undefined, async () => {
    assert.equal(typeof linearCredentials.accessToken, "function");
    await assert.rejects(linearCredentials.accessToken(), {
      message: "Linear credentials are unavailable."
    });
    assert.equal(await linearCredentials.webhookVerifier(request, "body"), null);
  });
});

test("Linear MCP requires its own connector configuration", async () => {
  await withEnvironment("LINEAR_MCP_CONNECT_UID", "  ", () => {
    assert.throws(linearMcpAuth, {
      message: "Missing required environment variable LINEAR_MCP_CONNECT_UID."
    });
  });
});

test("Linear MCP auth is app scoped for internal and scheduled sessions", async () => {
  await withEnvironment("LINEAR_MCP_CONNECT_UID", " mcp.linear.app/factory ", () => {
    const auth = linearMcpAuth();
    assert.equal(auth.principalType, "app");
    assert.equal(typeof auth.getToken, "function");
  });
});
