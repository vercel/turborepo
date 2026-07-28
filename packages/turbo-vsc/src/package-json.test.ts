import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const packageJson = JSON.parse(readFileSync("package.json", "utf8"));

test("keeps executable settings machine-scoped", () => {
  const properties = packageJson.contributes.configuration.properties;

  assert.equal(properties["turbo.path"].scope, "machine");
  assert.equal(properties["turbo.useLocalTurbo"].scope, "machine");
});

test("does not activate in untrusted workspaces", () => {
  assert.equal(packageJson.capabilities.untrustedWorkspaces.supported, false);
});
