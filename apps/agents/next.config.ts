import type { NextConfig } from "next";
import { withEve } from "eve/next";

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

export default withEve(nextConfig);
