"use client";

import {
  REMOTE_CACHE_MINUTES_SAVED_KEY,
  computeTimeSaved,
  remoteCacheTimeSavedQuery,
} from "./data";
import useSWR from "swr";

const counterFormatter = Intl.NumberFormat(undefined, {
  minimumIntegerDigits: 7,
  maximumFractionDigits: 0,
});

export const TheNumber = () => {
  const { data } = useSWR(
    REMOTE_CACHE_MINUTES_SAVED_KEY,
    async () => {
      const metrics = await remoteCacheTimeSavedQuery();
      return computeTimeSaved(metrics);
    },
    {
      refreshInterval: 5000,
      revalidateOnMount: true,
      onSuccess: () => console.log("success"),
      onError: () => console.log("error"),
    }
  );

  if (!data) {
    // TODO: Need to error ehre
    return <div>Loading...</div>;
  }

  return (
    <span className="inline-flex gap-1 text-xl text-transparent bg-gradient-to-r from-red-500 to-blue-500 bg-clip-text">
      {counterFormatter.format(data)} <p className="inline-block">hours</p>
    </span>
  );
};
