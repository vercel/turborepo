import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import type { CallToolResult } from "@modelcontextprotocol/sdk/types.js";
import { z } from "zod";

// registerTool validates arguments against this schema at runtime, so
// handlers receive properly typed numbers instead of trusting the caller.
const twoNumbers = {
  a: z.number(),
  b: z.number(),
};

function numberResult(value: number): CallToolResult {
  return { content: [{ type: "text", text: String(value) }] };
}

export function createServer(): McpServer {
  const server = new McpServer({
    name: "@repo/mcp-calculator",
    version: "1.0.0",
  });

  server.registerTool(
    "add",
    { description: "Add two numbers", inputSchema: twoNumbers },
    ({ a, b }) => numberResult(a + b),
  );

  server.registerTool(
    "subtract",
    { description: "Subtract b from a", inputSchema: twoNumbers },
    ({ a, b }) => numberResult(a - b),
  );

  server.registerTool(
    "multiply",
    { description: "Multiply two numbers", inputSchema: twoNumbers },
    ({ a, b }) => numberResult(a * b),
  );

  server.registerTool(
    "divide",
    { description: "Divide a by b", inputSchema: twoNumbers },
    ({ a, b }) => {
      if (b === 0) {
        // Execution failures are `isError` results (visible to the model),
        // not thrown errors (opaque JSON-RPC protocol errors).
        return {
          content: [{ type: "text", text: "Division by zero" }],
          isError: true,
        };
      }
      return numberResult(a / b);
    },
  );

  return server;
}
