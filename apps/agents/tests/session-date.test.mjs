import assert from "node:assert/strict";
import test from "node:test";

import { sessionDate } from "../agent/lib/repo.ts";

function encodeUlidTime(timestamp) {
  const alphabet = "0123456789ABCDEFGHJKMNPQRSTVWXYZ";
  let value = timestamp;
  let encoded = "";
  for (let index = 0; index < 10; index += 1) {
    encoded = alphabet[value % 32] + encoded;
    value = Math.floor(value / 32);
  }
  return encoded;
}

test("derives the runtime date from regular and workflow session ids", () => {
  const timestamp = Date.parse("2026-08-12T15:30:00.000Z");
  const encoded = encodeUlidTime(timestamp);
  assert.equal(
    sessionDate(`${encoded}REST`).toISOString(),
    "2026-08-12T15:30:00.000Z"
  );
  assert.equal(
    sessionDate(`wrun_${encoded}REST`).toISOString(),
    "2026-08-12T15:30:00.000Z"
  );
  assert.equal(
    sessionDate(`ses_${encoded}REST`).toISOString(),
    "2026-08-12T15:30:00.000Z"
  );
});
