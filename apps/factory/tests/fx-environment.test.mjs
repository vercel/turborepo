import assert from "node:assert/strict";
import test from "node:test";

import {
  fxEnvironment,
  listFxModels,
  parseFxModelCatalog
} from "../agent/lib/fx-environment.ts";

test("fx environment forwards Vercel OIDC authentication", () => {
  assert.deepEqual(fxEnvironment("oidc-token"), {
    FX_AUTO_UPGRADE: "0",
    FX_PERMISSION_MODE: "yolo",
    VERCEL_OIDC_TOKEN: "oidc-token"
  });
});

test("fx environment selects the workspace model", () => {
  assert.equal(
    fxEnvironment("oidc-token", "anthropic/claude-fable-5").FX_MODEL,
    "anthropic/claude-fable-5"
  );
});

test("parses the dynamic fx model catalog", () => {
  assert.deepEqual(
    parseFxModelCatalog({
      data: [
        { id: "new-provider/new-model", name: "New Model", type: "language" },
        { id: "new-provider/new-model", name: "Duplicate" },
        { id: "openai/gpt-5.6-sol" },
        { id: "invalid model", name: "Invalid" },
        null
      ]
    }),
    [
      { id: "new-provider/new-model", name: "New Model" },
      { id: "openai/gpt-5.6-sol", name: "openai/gpt-5.6-sol" }
    ]
  );
  assert.deepEqual(parseFxModelCatalog({ data: "nope" }), []);
});

test("loads the team-authenticated fx model catalog", async () => {
  const calls = [];
  const models = await listFxModels({
    oidcToken: "oidc-token",
    async fetch(url, init) {
      calls.push({ init, url });
      return Response.json({
        data: [{ id: "provider/dynamic-model", name: "Dynamic Model" }]
      });
    }
  });
  assert.deepEqual(models, [
    { id: "provider/dynamic-model", name: "Dynamic Model" }
  ]);
  assert.equal(
    calls[0].url,
    "https://ai-gateway.vercel.sh/coding-agent/v1/models"
  );
  assert.equal(calls[0].init.headers.authorization, "Bearer oidc-token");
});

test("retries the public fx catalog when authenticated discovery fails", async () => {
  let requests = 0;
  const models = await listFxModels({
    oidcToken: "oidc-token",
    async fetch() {
      requests += 1;
      return requests === 1
        ? new Response(null, { status: 401 })
        : Response.json({ data: [{ id: "provider/model", name: "Model" }] });
    }
  });
  assert.equal(requests, 2);
  assert.equal(models[0].id, "provider/model");
});
