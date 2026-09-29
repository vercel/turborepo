import assert from "node:assert/strict";
import test from "node:test";

import {
  buildResizeMessage,
  buildStartMessage,
  buildWebSocketUrl,
  DEFAULT_CWD,
  parseServerMessage
} from "../lib/sandbox-terminal-protocol.ts";

test("terminal protocol opens a shell in the Factory checkout", () => {
  assert.equal(
    buildWebSocketUrl("wss://example.com/pty", "token with spaces"),
    "wss://example.com/pty?token=token%20with%20spaces"
  );
  const start = buildStartMessage(100, 30);
  assert.equal(start.cwd, DEFAULT_CWD);
  assert.equal(start.cwd, "/factory/turborepo");
  assert.equal(start.cols, 100);
  assert.equal(start.rows, 30);
  assert.deepEqual(buildResizeMessage(120, 40), {
    cols: 120,
    rows: 40,
    type: "resize"
  });
});

test("terminal protocol distinguishes output and exit frames", () => {
  assert.deepEqual(parseServerMessage("hello"), {
    data: "hello",
    kind: "output"
  });
  assert.deepEqual(
    parseServerMessage(JSON.stringify({ code: 0, type: "exit" })),
    { code: 0, kind: "exit" }
  );
});
