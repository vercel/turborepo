import type { NextConfig } from "next";
import { withEve } from "eve/next";
import { withWorkflow } from "workflow/next";

const nextConfig: NextConfig = {
  poweredByHeader: false,
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
