import assert from "node:assert/strict";
import test from "node:test";

import { parseFxTurnResult } from "../agent/lib/fx-result.ts";

test("parses successful fx ask JSON", () => {
  assert.deepEqual(
    parseFxTurnResult(
      JSON.stringify({
        exit_code: 0,
        output: "Implemented the fix.",
        session_id: "1770000000000-1770000000000000000-a1b2c3d4e5f60718"
      }),
      0
    ),
    {
      output: "Implemented the fix.",
      sessionId: "1770000000000-1770000000000000000-a1b2c3d4e5f60718"
    }
  );
});

test("rejects failed and malformed fx output", () => {
  assert.equal(parseFxTurnResult("not json", 0), null);
  assert.equal(
    parseFxTurnResult(
      JSON.stringify({ exit_code: 1, output: "", session_id: "session" }),
      1
    ),
    null
  );
});
