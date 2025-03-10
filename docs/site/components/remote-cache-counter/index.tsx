"use client";

import { track } from "@vercel/analytics";
import Link from "next/link";
import { RemoteCacheCounterClient } from "./client";

export function RemoteCacheCounter(): JSX.Element {
  return (
    <Link
      className="group mt-4 h-[126px] min-h-[126px] w-full overflow-hidden rounded-lg border border-transparent bg-gradient-to-r from-red-500 to-blue-500 bg-origin-border text-[#6b7280] dark:text-[#9ca3af]"
      href="/repo/docs/core-concepts/remote-caching"
      onClick={() => track("Remote Cache counter click")}
    >
      <div className=" bg-white p-4 dark:bg-[#111111]">
        <span className="ml-auto inline-flex gap-1 bg-gradient-to-r from-red-500 to-blue-500 bg-clip-text font-mono text-lg text-transparent">
          <RemoteCacheCounterClient />
          <p className="inline-block">hours</p>
        </span>
        <div className="text-xs">Total Compute Saved</div>
        <div className="mt-4 text-xs group-hover:underline">
          Get started with
          <br /> Remote Caching →
        </div>
      </div>
    </Link>
  );
}
