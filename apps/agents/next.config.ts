import type { NextConfig } from "next";
import { withEve } from "eve/next";
import { withWorkflow } from "workflow/next";

const nextConfig: NextConfig = {
  poweredByHeader: false,
  serverExternalPackages: [
    "@ai-sdk/harness-claude-code",
    "@ai-sdk/harness-codex",
    "@ai-sdk/harness-opencode"
  ],
  async headers() {
    return [
      {
        source: "/:path*",
        headers: [{ key: "x-frame-options", value: "DENY" }]
      }
    ];
  }
};

export default withWorkflow(withEve(nextConfig));
