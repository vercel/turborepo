"use client";

import { animated } from "@react-spring/web";
import { useTurborepoMinutesSaved } from "./use-turborepo-minutes-saved";

const counterFormatter = Intl.NumberFormat(undefined, {
  minimumIntegerDigits: 7,
  maximumFractionDigits: 0,
});

export function RemoteCacheCounterClient(): JSX.Element {
  const timeSaved = useTurborepoMinutesSaved()?.total;

  return (
    <>
      {timeSaved ? (
        <animated.p className="inline-block tabular-nums">
          {counterFormatter.format(timeSaved / 60)}
        </animated.p>
      ) : (
        <p className="h-5 w-[97.2px]" />
      )}
    </>
  );
}
