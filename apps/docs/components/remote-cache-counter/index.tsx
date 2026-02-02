"use client";

import { track } from "@vercel/analytics";
import Link from "next/link";
import { RemoteCacheCounterClient } from "./client";

export function RemoteCacheCounter() {
  return (
    <Link
      className="group my-4 h-[126px] min-h-[126px] w-full overflow-hidden rounded-lg border border-transparent bg-gradient-to-r from-[#ef4444] to-[#3b82f6] bg-origin-border"
      href="/docs/core-concepts/remote-caching"
      onClick={() => {
        track("Remote Cache counter click");
      }}
    >
      <div className="bg-background p-4">
        <span className="ml-auto inline-flex gap-1 bg-gradient-to-r from-[#ef4444] to-[#3b82f6] bg-clip-text font-mono text-lg text-transparent">
          <RemoteCacheCounterClient className="min-w-[97.2px] text-right" />
          <p className="inline-block">hours</p>
        </span>
        <div className="text-sm text-muted-foreground">Total Compute Saved</div>
        <div className="mt-2 text-sm text-muted-foreground transition-colors group-hover:text-foreground">
          Get started with
          <br /> Remote Caching →
        </div>
      </div>
    </Link>
  );
}
