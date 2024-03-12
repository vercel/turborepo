"use client";

import { animated, useSpring, config } from "@react-spring/web";
import {
  REMOTE_CACHE_MINUTES_SAVED_URL,
  REMOTE_CACHE_TIME_TAG,
  computeTimeSaved,
  fetchTimeSaved,
} from "./data";
import useSWR from "swr";
import { useState } from "react";
import { revalidateTag } from "next/cache";
import { revalidateRemoteCacheMetrics } from "@/components/RemoteCacheCounterButRsc/revalidateData";

const counterFormatter = Intl.NumberFormat(undefined, {
  minimumIntegerDigits: 7,
  maximumFractionDigits: 0,
});

export const TheNumber = ({ startingNumber }: { startingNumber: number }) => {
  const [targetHours, setTargetHours] = useState(startingNumber);

  const { data } = useSWR(
    REMOTE_CACHE_MINUTES_SAVED_URL,
    async () => {
      const metrics = await fetchTimeSaved(REMOTE_CACHE_MINUTES_SAVED_URL);
      revalidateRemoteCacheMetrics();
      const computed = computeTimeSaved(metrics);

      setTargetHours(computed);
      return computed;
    },
    {
      refreshInterval: 5000,
      revalidateOnMount: false,
      revalidateOnFocus: false,
      revalidateOnReconnect: false,
      onSuccess: () => console.log("success"),
      onError: (err) => {
        console.log("error", err);
      },
    }
  );

  const spring = useSpring({
    from: { hoursSaved: startingNumber },
    hoursSaved: targetHours,
    config: config.molasses,
  });

  if (!data) {
    // TODO: Need to error here
    return <div>Loading...</div>;
  }

  return (
    <span className="inline-flex gap-1 text-xl text-transparent bg-gradient-to-r from-red-500 to-blue-500 bg-clip-text">
      <animated.p className="inline-block tabular-nums">
        {spring.hoursSaved.to((t) => counterFormatter.format(t))}
      </animated.p>
      <p className="inline-block">hours</p>
    </span>
  );
};
