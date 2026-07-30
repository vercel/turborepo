import assert from "node:assert/strict";
import { test } from "node:test";
import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { InMemoryTransport } from "@modelcontextprotocol/sdk/inMemory.js";
import type { CallToolResult } from "@modelcontextprotocol/sdk/types.js";
import { createServer } from "./server.js";

async function connect(): Promise<Client> {
  const [clientTransport, serverTransport] =
    InMemoryTransport.createLinkedPair();
  const client = new Client({ name: "test-client", version: "1.0.0" });
  await Promise.all([
    createServer().connect(serverTransport),
    client.connect(clientTransport),
  ]);
  return client;
}

async function callTool(
  client: Client,
  name: string,
  args: Record<string, unknown>,
): Promise<CallToolResult> {
  return (await client.callTool({
    name,
    arguments: args,
  })) as CallToolResult;
}

function firstText(result: CallToolResult): string {
  const [first] = result.content;
  assert.equal(first?.type, "text");
  return first.type === "text" ? first.text : "";
}

test("performs the four arithmetic operations", async () => {
  const client = await connect();
  try {
    const cases = [
      { name: "add", a: 5, b: 3, expected: "8" },
      { name: "subtract", a: 5, b: 3, expected: "2" },
      { name: "multiply", a: 6, b: 7, expected: "42" },
      { name: "divide", a: 10, b: 4, expected: "2.5" },
    ];
    for (const { name, a, b, expected } of cases) {
      const result = await callTool(client, name, { a, b });
      assert.equal(result.isError, undefined);
      assert.equal(firstText(result), expected);
    }
  } finally {
    await client.close();
  }
});

test("returns an isError result for division by zero", async () => {
  const client = await connect();
  try {
    const result = await callTool(client, "divide", { a: 1, b: 0 });
    assert.equal(result.isError, true);
    assert.equal(firstText(result), "Division by zero");
  } finally {
    await client.close();
  }
});

// The SDK validates arguments against the registered zod schema and returns
// failures as isError results, so handlers never see bad input (no "5" + "3"
// string concatenation, no NaN from missing arguments).
test("returns an isError result for non-numeric arguments", async () => {
  const client = await connect();
  try {
    const result = await callTool(client, "add", { a: "5", b: "3" });
    assert.equal(result.isError, true);
    assert.match(firstText(result), /Invalid arguments/);
  } finally {
    await client.close();
  }
});

test("returns an isError result for missing arguments", async () => {
  const client = await connect();
  try {
    const result = await callTool(client, "add", { a: 5 });
    assert.equal(result.isError, true);
    assert.match(firstText(result), /Invalid arguments/);
  } finally {
    await client.close();
  }
});

test("returns an isError result for unknown tools, including inherited property names", async () => {
  const client = await connect();
  try {
    const modulo = await callTool(client, "modulo", { a: 1, b: 2 });
    assert.equal(modulo.isError, true);
    const toString = await callTool(client, "toString", { a: 1, b: 2 });
    assert.equal(toString.isError, true);
  } finally {
    await client.close();
  }
});
