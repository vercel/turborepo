"use client";

import { animated } from "@react-spring/web";
import {
  REMOTE_CACHE_MINUTES_SAVED_URL,
  computeTimeSaved,
  fetchTimeSaved,
} from "./data";
import useSWR from "swr";
import { serverRevalidator } from "@/components/RemoteCacheCounterButRsc/server-revalidator";

const counterFormatter = Intl.NumberFormat(undefined, {
  minimumIntegerDigits: 7,
  maximumFractionDigits: 0,
});

export const TheNumber = () => {
  const { data, error } = useSWR(
    REMOTE_CACHE_MINUTES_SAVED_URL,
    async () => {
      const metrics = await fetchTimeSaved(REMOTE_CACHE_MINUTES_SAVED_URL);
      serverRevalidator();
      return computeTimeSaved(metrics);
    },
    {
      refreshInterval: 5000,
      revalidateOnMount: false,
      revalidateOnFocus: false,
      revalidateOnReconnect: false,
    }
  );

  // const spring = useSpring({
  //   from: { hoursSaved: startingNumber },
  //   hoursSaved: targetHours,
  //   config: config.molasses,
  // });

  if (error) {
    // TODO: Need to handle errors?
    return <div>ERRORS!</div>;
  }

  return (
    <span className="inline-flex gap-1 text-xl text-transparent bg-gradient-to-r from-red-500 to-blue-500 bg-clip-text">
      <animated.p className="inline-block tabular-nums">
        {counterFormatter.format(data!)}
        {/* {spring.hoursSaved.to((t) => counterFormatter.format(t))} */}
      </animated.p>
      <p className="inline-block">hours</p>
    </span>
  );
};
