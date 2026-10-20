#!/usr/bin/env node
// Executable entry: launches the calculator server over stdio. The library
// entry (server.ts) is side-effect free so it can be imported and tested.
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { createServer } from "./server.js";

await createServer().connect(new StdioServerTransport());
