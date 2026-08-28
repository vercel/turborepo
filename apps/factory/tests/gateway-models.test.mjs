import assert from "node:assert/strict";
import test from "node:test";

import { parseGatewayModels } from "../agent/lib/gateway-models.ts";

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
