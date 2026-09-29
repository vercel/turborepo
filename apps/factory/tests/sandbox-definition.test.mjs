import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

test("the sandbox backend tolerates a missing runtime image handoff", () => {
  const source = readFileSync(new URL("../agent/sandbox.ts", import.meta.url), {
    encoding: "utf8"
  });

  assert.match(source, /handoff === null\s*\? \{/);
  assert.doesNotMatch(source, /if \(handoff === null\) \{\s*throw new Error/);
});
