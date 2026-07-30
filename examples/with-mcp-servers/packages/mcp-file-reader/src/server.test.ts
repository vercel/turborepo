import assert from "node:assert/strict";
import { mkdir, mkdtemp, rm, symlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { after, before, test } from "node:test";
import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { InMemoryTransport } from "@modelcontextprotocol/sdk/inMemory.js";
import type { CallToolResult } from "@modelcontextprotocol/sdk/types.js";
import { createServer, MAX_FILE_SIZE_BYTES } from "./server.js";

let fixtureDir: string;
let rootDir: string;

before(async () => {
  fixtureDir = await mkdtemp(join(tmpdir(), "mcp-file-reader-test-"));
  rootDir = join(fixtureDir, "root");
  await mkdir(rootDir);
  await writeFile(join(rootDir, "file.txt"), "hello");
  await writeFile(
    join(rootDir, "large.txt"),
    "x".repeat(MAX_FILE_SIZE_BYTES + 1),
  );
  // A secret outside the allowed root, plus a symlink inside the root that
  // points at it. Neither must be readable.
  await writeFile(join(fixtureDir, "secret.txt"), "top secret");
  await symlink(join(fixtureDir, "secret.txt"), join(rootDir, "escape.txt"));
});

after(async () => {
  await rm(fixtureDir, { recursive: true, force: true });
});

async function connect(): Promise<Client> {
  const [clientTransport, serverTransport] =
    InMemoryTransport.createLinkedPair();
  const client = new Client({ name: "test-client", version: "1.0.0" });
  await Promise.all([
    createServer(rootDir).connect(serverTransport),
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

test("reads a file within the root", async () => {
  const client = await connect();
  try {
    const result = await callTool(client, "read_file", { path: "file.txt" });
    assert.equal(result.isError, undefined);
    assert.equal(firstText(result), "hello");
  } finally {
    await client.close();
  }
});

test("rejects relative path traversal outside the root", async () => {
  const client = await connect();
  try {
    const result = await callTool(client, "read_file", {
      path: "../secret.txt",
    });
    assert.equal(result.isError, true);
    assert.match(firstText(result), /outside the allowed root/);
  } finally {
    await client.close();
  }
});

test("rejects absolute paths outside the root", async () => {
  const client = await connect();
  try {
    const result = await callTool(client, "read_file", {
      path: join(fixtureDir, "secret.txt"),
    });
    assert.equal(result.isError, true);
    assert.match(firstText(result), /outside the allowed root/);
  } finally {
    await client.close();
  }
});

test("rejects symlinks that escape the root", async () => {
  const client = await connect();
  try {
    const result = await callTool(client, "read_file", {
      path: "escape.txt",
    });
    assert.equal(result.isError, true);
    assert.match(firstText(result), /outside the allowed root/);
  } finally {
    await client.close();
  }
});

test("rejects files over the size limit", async () => {
  const client = await connect();
  try {
    const result = await callTool(client, "read_file", { path: "large.txt" });
    assert.equal(result.isError, true);
    assert.match(firstText(result), /too large/);
  } finally {
    await client.close();
  }
});

test("returns an isError result for missing files without leaking resolved paths", async () => {
  const client = await connect();
  try {
    const result = await callTool(client, "read_file", {
      path: "missing.txt",
    });
    assert.equal(result.isError, true);
    assert.equal(firstText(result), "ENOENT: missing.txt");
  } finally {
    await client.close();
  }
});

test("lists a directory within the root", async () => {
  const client = await connect();
  try {
    const result = await callTool(client, "list_directory", { path: "." });
    assert.equal(result.isError, undefined);
    const listing = JSON.parse(firstText(result)) as {
      entries: string[];
      truncated: boolean;
    };
    assert.ok(listing.entries.includes("file.txt"));
    assert.equal(listing.truncated, false);
  } finally {
    await client.close();
  }
});

test("rejects listing directories outside the root", async () => {
  const client = await connect();
  try {
    const result = await callTool(client, "list_directory", { path: ".." });
    assert.equal(result.isError, true);
    assert.match(firstText(result), /outside the allowed root/);
  } finally {
    await client.close();
  }
});
