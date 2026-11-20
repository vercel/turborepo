import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { StdioClientTransport } from "@modelcontextprotocol/sdk/client/stdio.js";
import type { CallToolResult } from "@modelcontextprotocol/sdk/types.js";
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";

// Each server package exposes its compiled executable via a `./cli` export.
// Resolving it through the package manager (instead of a hardcoded relative
// path) keeps this working no matter where the package is installed, and
// Turbo's `^build` dependency guarantees the compiled output exists.
const require = createRequire(import.meta.url);

async function connectServer(
  cliSpecifier: string,
  args: string[] = [],
): Promise<Client> {
  const transport = new StdioClientTransport({
    command: process.execPath,
    args: [require.resolve(cliSpecifier), ...args],
  });
  const client = new Client(
    { name: "mcp-client", version: "1.0.0" },
    { capabilities: {} },
  );
  await client.connect(transport);
  return client;
}

async function callTextTool(
  client: Client,
  name: string,
  args: Record<string, unknown>,
): Promise<string> {
  const result = (await client.callTool({
    name,
    arguments: args,
  })) as CallToolResult;
  const [first] = result.content;
  if (first?.type !== "text") {
    throw new Error(`Tool "${name}" did not return text content`);
  }
  if (result.isError) {
    throw new Error(`Tool "${name}" failed: ${first.text}`);
  }
  return first.text;
}

async function main(): Promise<void> {
  // This file runs from dist/, so: dist -> mcp-client -> apps -> example root.
  const exampleRoot = fileURLToPath(new URL("../../..", import.meta.url));

  // Each connection spawns a server as a child process. Connecting one at a
  // time keeps cleanup simple; a production host would connect concurrently.
  const clients: Client[] = [];
  try {
    const calc = await connectServer("@repo/mcp-calculator/cli");
    clients.push(calc);

    // The file-reader takes its allowed root directory as an argument; it
    // refuses to read anything outside of it.
    const fileReader = await connectServer("@repo/mcp-file-reader/cli", [
      exampleRoot,
    ]);
    clients.push(fileReader);

    const sum = await callTextTool(calc, "add", { a: 5, b: 3 });
    console.log(`5 + 3 = ${sum}`);

    const product = await callTextTool(calc, "multiply", { a: 6, b: 7 });
    console.log(`6 × 7 = ${product}`);

    const listing = await callTextTool(fileReader, "list_directory", {
      path: ".",
    });
    console.log(`Example root: ${listing}`);
  } finally {
    // Always close connected clients so the spawned server processes exit,
    // even when connecting or a tool call fails.
    await Promise.all(clients.map((client) => client.close()));
  }
}

main().catch((error: unknown) => {
  console.error(error);
  process.exitCode = 1;
});
