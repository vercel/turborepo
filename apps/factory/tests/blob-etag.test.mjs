import assert from "node:assert/strict";
import test from "node:test";

import { strongBlobEtag } from "../agent/lib/blob-etag.ts";

test("normalizes Blob get ETags for conditional writes", () => {
  assert.equal(strongBlobEtag('W/"abc123"'), '"abc123"');
  assert.equal(strongBlobEtag('"abc123"'), '"abc123"');
});
