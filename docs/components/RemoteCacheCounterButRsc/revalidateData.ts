"use server";

import { REMOTE_CACHE_TIME_TAG } from "@/components/RemoteCacheCounterButRsc/data";
import { revalidateTag } from "next/cache";

export async function revalidateRemoteCacheMetrics() {
  revalidateTag(REMOTE_CACHE_TIME_TAG);
}
