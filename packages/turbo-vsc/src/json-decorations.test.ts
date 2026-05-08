import assert from "node:assert/strict";
import test from "node:test";

import { getTaskDefinitionKeyDecorationOffsets } from "./json-decorations";

function keyOffsets(json: string, key: "pipeline" | "tasks"): number[] {
  const start = json.indexOf(key);
  return Array.from({ length: key.length }, (_, i) => start + i);
}

test("decorates top-level tasks without throwing", () => {
  const json = `{"tasks":{"build":{}}}`;

  assert.deepEqual(
    getTaskDefinitionKeyDecorationOffsets(json),
    keyOffsets(json, "tasks")
  );
});

test("decorates legacy top-level pipeline", () => {
  const json = `{"pipeline":{"build":{}}}`;

  assert.deepEqual(
    getTaskDefinitionKeyDecorationOffsets(json),
    keyOffsets(json, "pipeline")
  );
});

test("does not decorate nested task definition keys", () => {
  assert.deepEqual(
    getTaskDefinitionKeyDecorationOffsets(
      `{"config":{"tasks":{},"pipeline":{}}}`
    ),
    []
  );
});

test("returns to top-level after a sibling object", () => {
  const json = `{"globalEnv":{},"tasks":{}}`;

  assert.deepEqual(
    getTaskDefinitionKeyDecorationOffsets(json),
    keyOffsets(json, "tasks")
  );
});

test("prefers tasks over pipeline", () => {
  const json = `{"pipeline":{},"tasks":{}}`;

  assert.deepEqual(
    getTaskDefinitionKeyDecorationOffsets(json),
    keyOffsets(json, "tasks")
  );
});
