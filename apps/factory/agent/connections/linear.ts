import { defineMcpClientConnection } from "eve/connections";
import { once } from "eve/tools/approval";

import { linearMcpAuth } from "../lib/linear.js";

export default defineMcpClientConnection({
  url: "https://mcp.linear.app/mcp",
  description:
    "Linear workspace: search and update issues, projects, cycles, and comments.",
  auth: linearMcpAuth,
  // Require session consent before exposing the shared workspace's tools.
  approval: once()
});
