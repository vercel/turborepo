import assert from "node:assert/strict";
import test from "node:test";

import {
  buildStartMessage,
  buildResizeMessage,
  buildWebSocketUrl,
  parseServerMessage,
  TERM,
  PS1,
  DEFAULT_CWD
} from "../lib/sandbox-terminal-protocol.ts";

test("buildWebSocketUrl appends the token as a query parameter", () => {
  assert.equal(
    buildWebSocketUrl("wss://example.com/ws", "tok"),
    "wss://example.com/ws?token=tok"
  );
  assert.equal(
    buildWebSocketUrl("wss://example.com/ws?foo=1", "tok"),
    "wss://example.com/ws?foo=1&token=tok"
  );
  assert.equal(
    buildWebSocketUrl("wss://example.com/ws", "tok with spaces"),
    "wss://example.com/ws?token=tok%20with%20spaces"
  );
});

test("buildStartMessage includes required fields and defaults", () => {
  const message = buildStartMessage(80, 24);
  assert.equal(message.type, "start");
  assert.equal(message.command, "sh");
  assert.deepEqual(message.args, []);
  assert.equal(message.cwd, DEFAULT_CWD);
  assert.equal(message.cols, 80);
  assert.equal(message.rows, 24);
  assert.ok(message.env.includes(`TERM=${TERM}`));
  assert.ok(message.env.includes(`PS1=${PS1}`));
});

test("buildStartMessage accepts command, args, env, and cwd overrides", () => {
  const message = buildStartMessage(100, 30, {
    command: "bash",
    args: ["-l"],
    env: { FOO: "bar" },
    cwd: "/workspace"
  });
  assert.equal(message.command, "bash");
  assert.deepEqual(message.args, ["-l"]);
  assert.equal(message.cwd, "/workspace");
  assert.equal(message.cols, 100);
  assert.equal(message.rows, 30);
  assert.ok(message.env.includes("FOO=bar"));
  assert.ok(message.env.includes(`TERM=${TERM}`));
});

test("buildResizeMessage encodes a resize control frame", () => {
  assert.deepEqual(buildResizeMessage(120, 40), {
    type: "resize",
    cols: 120,
    rows: 40
  });
});

test("parseServerMessage parses exit control frames", () => {
  assert.deepEqual(
    parseServerMessage(JSON.stringify({ type: "exit", code: 0 })),
    {
      kind: "exit",
      code: 0
    }
  );
  assert.deepEqual(
    parseServerMessage(JSON.stringify({ type: "exit", code: 1 })),
    {
      kind: "exit",
      code: 1
    }
  );
  assert.deepEqual(
    parseServerMessage(JSON.stringify({ type: "exit", code: null })),
    {
      kind: "exit",
      code: null
    }
  );
});

test("parseServerMessage treats text and binary data as output", () => {
  assert.deepEqual(parseServerMessage("hello"), {
    kind: "output",
    data: "hello"
  });
  const buffer = new TextEncoder().encode("binary data").buffer;
  assert.deepEqual(parseServerMessage(buffer), {
    kind: "output",
    data: "binary data"
  });
});

test("parseServerMessage treats non-JSON text as output", () => {
  assert.deepEqual(parseServerMessage("not json"), {
    kind: "output",
    data: "not json"
  });
});

test("parseServerMessage marks blobs as unknown", () => {
  const blob = new Blob(["data"]);
  const result = parseServerMessage(blob);
  assert.equal(result.kind, "unknown");
});
