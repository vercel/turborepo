#!/usr/bin/env node
// Executable entry: launches the file-reader server over stdio. The library
// entry (server.ts) is side-effect free so it can be imported and tested.
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { createServer } from "./server.js";

// The allowed root directory is the server's first CLI argument, defaulting
// to the working directory. Reads outside this root are rejected.
const rootDir = process.argv[2] ?? process.cwd();

await createServer(rootDir).connect(new StdioServerTransport());
