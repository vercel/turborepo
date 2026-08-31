import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import {
  FACTORY_HARNESS_PORT,
  FACTORY_HARNESS_WORKDIR
} from "../agent/lib/harness-agent-config.ts";
import { parseHarnessResumeState } from "../agent/lib/harness-agent-state.ts";

const bridgePackages = [
  "@ai-sdk/harness-acp",
  "@ai-sdk/harness-claude-code",
  "@ai-sdk/harness-codex",
  "@ai-sdk/harness-opencode"
];

test("HarnessAgent uses the exposed Factory sandbox port and checkout", () => {
  assert.equal(FACTORY_HARNESS_PORT, 4000);
  assert.equal(FACTORY_HARNESS_WORKDIR, "turborepo");
});

test("HarnessAgent bridge adapters preserve their package-relative assets", () => {
  const agent = readFileSync(
    new URL("../agent/agent.ts", import.meta.url),
    "utf8"
  );
  const manifest = JSON.parse(
    readFileSync(new URL("../package.json", import.meta.url), "utf8")
  );

  for (const packageName of bridgePackages) {
    assert.match(agent, new RegExp(`"${packageName}"`));
    assert.ok(
      manifest.dependencies[packageName],
      `${packageName} must be a direct dependency to remain external`
    );
  }
  assert.match(agent, /externalDependencies:/);
  assert.match(manifest.scripts.build, /stage-harness-assets\.mjs/);
});

test("HarnessAgent resume state rejects malformed persisted values", () => {
  const state = { harnessId: "fx", lifecycleState: { sessionId: "fx_123" } };
  assert.deepEqual(parseHarnessResumeState(JSON.stringify(state)), state);
  assert.equal(parseHarnessResumeState("not json"), undefined);
  assert.equal(parseHarnessResumeState("null"), undefined);
  assert.equal(parseHarnessResumeState('"state"'), undefined);
});
