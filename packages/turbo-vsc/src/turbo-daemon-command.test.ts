import assert from "node:assert/strict";
import test from "node:test";

import { createTurboDaemonArgs } from "./turbo-daemon-command";

test("passes daemon commands as argv", () => {
  assert.deepEqual(createTurboDaemonArgs("start"), ["daemon", "start"]);
  assert.deepEqual(createTurboDaemonArgs("stop"), ["daemon", "stop"]);
  assert.deepEqual(createTurboDaemonArgs("status"), ["daemon", "status"]);
});
