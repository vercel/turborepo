import assert from "node:assert/strict";
import test from "node:test";

import { fxEnvironment } from "../agent/lib/fx-environment.ts";

test("fx environment forwards Vercel OIDC authentication", () => {
  assert.deepEqual(fxEnvironment("oidc-token"), {
    FX_AUTO_UPGRADE: "0",
    FX_PERMISSION_MODE: "yolo",
    VERCEL_OIDC_TOKEN: "oidc-token"
  });
});
