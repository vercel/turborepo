import { readdir, readFile, realpath, stat } from "node:fs/promises";
import { isAbsolute, relative, resolve } from "node:path";
import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import type { CallToolResult } from "@modelcontextprotocol/sdk/types.js";
import { z } from "zod";

/** Refuse to read files larger than this to keep tool responses bounded. */
export const MAX_FILE_SIZE_BYTES = 1024 * 1024;

/** Cap directory listings so huge directories don't produce huge responses. */
export const MAX_DIRECTORY_ENTRIES = 1000;

function textResult(text: string): CallToolResult {
  return { content: [{ type: "text", text }] };
}

function errorResult(text: string): CallToolResult {
  // Tool execution failures are returned as `isError` results (not thrown)
  // so the model can see what went wrong and react. Thrown errors become
  // opaque JSON-RPC protocol errors instead.
  return { content: [{ type: "text", text }], isError: true };
}

/**
 * Reports errors using the caller-supplied path rather than raw error
 * messages, which contain resolved absolute paths from the host machine.
 */
function describeError(error: unknown, path: string): string {
  if (
    error instanceof Error &&
    "code" in error &&
    typeof error.code === "string"
  ) {
    return `${error.code}: ${path}`;
  }
  return error instanceof Error ? error.message : `Operation failed: ${path}`;
}

/**
 * Creates the file-reader MCP server, confined to `rootDir`.
 *
 * MCP tool arguments come from the model, so they are untrusted input.
 * Every path is resolved against the root and rejected if it escapes it,
 * including escapes through symlinks.
 */
export function createServer(rootDir: string): McpServer {
  const root = resolve(rootDir);

  async function resolveWithinRoot(path: string): Promise<string> {
    // realpath resolves symlinks in both the root and the target, so a
    // symlink inside the root can't point reads outside of it.
    const realRoot = await realpath(root);
    const resolved = await realpath(resolve(realRoot, path));
    const relativePath = relative(realRoot, resolved);
    if (relativePath.startsWith("..") || isAbsolute(relativePath)) {
      throw new Error(`Path is outside the allowed root: ${path}`);
    }
    return resolved;
  }

  const server = new McpServer({
    name: "@repo/mcp-file-reader",
    version: "1.0.0",
  });

  server.registerTool(
    "read_file",
    {
      description: `Read the contents of a file (up to ${MAX_FILE_SIZE_BYTES} bytes) within the allowed root directory`,
      inputSchema: { path: z.string().describe("Path to the file") },
    },
    async ({ path }) => {
      try {
        const resolved = await resolveWithinRoot(path);
        const { size } = await stat(resolved);
        if (size > MAX_FILE_SIZE_BYTES) {
          return errorResult(
            `File is too large to read (${size} bytes, limit is ${MAX_FILE_SIZE_BYTES}): ${path}`,
          );
        }
        return textResult(await readFile(resolved, "utf-8"));
      } catch (error) {
        return errorResult(describeError(error, path));
      }
    },
  );

  server.registerTool(
    "list_directory",
    {
      description:
        "List the entries in a directory within the allowed root directory",
      inputSchema: { path: z.string().describe("Path to the directory") },
    },
    async ({ path }) => {
      try {
        const resolved = await resolveWithinRoot(path);
        const entries = await readdir(resolved);
        const truncated = entries.length > MAX_DIRECTORY_ENTRIES;
        return textResult(
          JSON.stringify({
            entries: entries.slice(0, MAX_DIRECTORY_ENTRIES),
            truncated,
          }),
        );
      } catch (error) {
        return errorResult(describeError(error, path));
      }
    },
  );

  return server;
}
