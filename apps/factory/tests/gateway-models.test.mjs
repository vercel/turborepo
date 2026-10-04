import assert from "node:assert/strict";
import test from "node:test";

import {
  fastModelOption,
  parseGatewayModels
} from "../agent/lib/gateway-models.ts";

test("keeps and sorts tool-capable language models", () => {
  assert.deepEqual(
    parseGatewayModels({
      data: [
        {
          id: "provider/zeta",
          name: "Zeta",
          owned_by: "provider",
          supported_parameters: ["tools"],
          type: "language"
        },
        {
          id: "provider/alpha",
          name: "Alpha",
          owned_by: "provider",
          supported_parameters: ["tools", "reasoning"],
          type: "language"
        },
        {
          id: "provider/no-tools",
          name: "No tools",
          owned_by: "provider",
          supported_parameters: ["reasoning"],
          type: "language"
        },
        {
          id: "provider/image",
          name: "Image",
          owned_by: "provider",
          supported_parameters: ["tools"],
          type: "image"
        }
      ]
    }),
    [
      { id: "provider/alpha", name: "Alpha", ownedBy: "provider" },
      { id: "provider/zeta", name: "Zeta", ownedBy: "provider" }
    ]
  );
});

test("returns an empty list for an invalid response", () => {
  assert.deepEqual(parseGatewayModels(null), []);
  assert.deepEqual(parseGatewayModels({ data: "invalid" }), []);
});

test("fast mode selects only an available catalog variant in either direction", () => {
  const standard = { id: "openai/gpt-6.1-sol" };
  const fast = { id: "openai/gpt-6.1-sol-fast" };
  const models = [standard, fast];
  assert.equal(fastModelOption(models, standard.id), fast);
  assert.equal(fastModelOption(models, fast.id), standard);
  assert.equal(fastModelOption([standard], standard.id), undefined);
  assert.equal(fastModelOption([], standard.id), undefined);
});

test("keeps Fast variants in the model picker catalog", () => {
  const id = "openai/gpt-6.1-sol-fast";
  assert.deepEqual(
    parseGatewayModels({
      data: [
        {
          id,
          name: "GPT-6.1 Sol (Fast)",
          owned_by: "openai",
          supported_parameters: ["tools"],
          type: "language"
        }
      ]
    }),
    [{ id, name: "GPT-6.1 Sol (Fast)", ownedBy: "openai" }]
  );
});
