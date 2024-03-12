"use server";

import { REMOTE_CACHE_METRIC_TAG } from "@/components/RemoteCacheCounterButRsc/data";
import { revalidateTag } from "next/cache";

export const serverRevalidator = async () => {
  console.log("reavling");
  revalidateTag(REMOTE_CACHE_METRIC_TAG);
};
