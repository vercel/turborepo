import assert from "node:assert/strict";
import test from "node:test";

import { linearCredentials } from "../agent/lib/linear.ts";

function withEnvironment(values, run) {
  const names = ["LINEAR_CONNECT_UID", "LINEAR_INSTALLATION_ID"];
  const saved = new Map(names.map((name) => [name, process.env[name]]));
  for (const name of names) {
    if (values[name] === undefined) delete process.env[name];
    else process.env[name] = values[name];
  }
  return run().finally(() => {
    for (const [name, value] of saved) {
      if (value === undefined) delete process.env[name];
      else process.env[name] = value;
    }
  });
}

test("access token resolution fails closed without a connector UID", () =>
  withEnvironment({}, async () => {
    await assert.rejects(
      async () => {
        const accessToken = linearCredentials.accessToken;
        assert.equal(typeof accessToken, "function");
        await accessToken();
      },
      { message: "Linear credentials are unavailable." }
    );
  }));

test("treats a whitespace-only connector UID as missing", () =>
  withEnvironment({ LINEAR_CONNECT_UID: "   " }, async () => {
    await assert.rejects(async () => linearCredentials.accessToken(), {
      message: "Linear credentials are unavailable."
    });
  }));

test("webhook verification rejects requests without a connector UID", () =>
  withEnvironment({}, async () => {
    const verifier = linearCredentials.webhookVerifier;
    assert.equal(typeof verifier, "function");
    const verdict = await verifier(
      new Request("https://factory.test/eve/v1/linear", { method: "POST" }),
      "{}"
    );
    // A falsy verdict tells the channel to reject the request.
    assert.equal(verdict, null);
  }));
