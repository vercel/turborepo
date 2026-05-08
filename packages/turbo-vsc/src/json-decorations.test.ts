import assert from "node:assert/strict";
import test from "node:test";

import { getPipelineDecorationOffsets } from "./json-decorations";

function pipelineOffsets(json: string): number[] {
  const start = json.indexOf("pipeline");
  return Array.from({ length: "pipeline".length }, (_, i) => start + i);
}

test("decorates top-level pipeline without throwing", () => {
  const json = `{"pipeline":{"build":{}}}`;

  assert.deepEqual(getPipelineDecorationOffsets(json), pipelineOffsets(json));
});

test("does not decorate nested pipeline", () => {
  assert.deepEqual(
    getPipelineDecorationOffsets(`{"tasks":{"pipeline":{}}}`),
    []
  );
});

test("returns to top-level after a sibling object", () => {
  const json = `{"tasks":{},"pipeline":{}}`;

  assert.deepEqual(getPipelineDecorationOffsets(json), pipelineOffsets(json));
});
