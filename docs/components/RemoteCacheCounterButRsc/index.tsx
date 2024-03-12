import Link from "next/link";
import { TheNumber } from "./TheNumber";
import { SwrProvider } from "./swr-provider";
import {
  REMOTE_CACHE_MINUTES_SAVED_URL,
  computeTimeSaved,
  remoteCacheTimeSavedQuery,
} from "@/components/RemoteCacheCounterButRsc/data";

export async function RemoteCacheCounterButRsc(): Promise<JSX.Element> {
  const startingAnimationNumber =
    computeTimeSaved(
      await remoteCacheTimeSavedQuery(REMOTE_CACHE_MINUTES_SAVED_URL)
    ) - 50;

  return (
    <Link
      className="group w-full mt-4 rounded-lg border border-transparent overflow-hidden bg-origin-border bg-gradient-to-r from-red-500 to-blue-500 dark:text-[#9ca3af] text-[#6b7280]"
      href="/repo/docs/core-concepts/remote-caching"
    >
      <div className="p-4 dark:bg-[#111111] bg-white">
        <TheNumber startingNumber={startingAnimationNumber} />
        <div className="text-xs">Total Compute Saved</div>
        <div className="mt-4 text-xs group-hover:underline">
          Get started with
          <br /> Remote Caching →
        </div>
      </div>
    </Link>
  );
}
