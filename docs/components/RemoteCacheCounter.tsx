import { useState, useEffect } from "react";
import { animated, useSpring, config } from "@react-spring/web";
import Link from "next/link";
import { useTurborepoMinutesSaved } from "../lib/useTurborepoMinutesSaved";

const counterFormatter = Intl.NumberFormat(undefined, {
  minimumIntegerDigits: 7,
  maximumFractionDigits: 0,
});

// Arbitrary number based on the value on January 25th, 2024.
// If the number takes too long to update on page load, you can bump this number up.
const START_NUM = 2080300;

export function RemoteCacheCounter() {
  const [targetHours, setTargetHours] = useState(START_NUM);
  const timeSaved = useTurborepoMinutesSaved();
  useEffect(() => {
    if (timeSaved) {
      setTargetHours(
        (timeSaved.local_cache_minutes_saved +
          timeSaved.remote_cache_minutes_saved) /
          60
      );
    }
  }, [timeSaved]);

  const spring = useSpring({
    from: { hoursSaved: START_NUM },
    hoursSaved: targetHours,
    config: config.molasses,
  });

  return (
    <Link
      className="group w-full mt-4 rounded-lg border border-transparent overflow-hidden bg-origin-border bg-gradient-to-r from-red-500 to-blue-500 dark:text-[#9ca3af] text-[#6b7280]"
      href="/repo/docs/core-concepts/remote-caching"
    >
      <div className="p-4 dark:bg-[#111111] bg-white">
        <span className="inline-flex gap-1 text-xl text-transparent bg-gradient-to-r from-red-500 to-blue-500 bg-clip-text">
          <animated.p className="inline-block tabular-nums">
            {spring.hoursSaved.to((t) => counterFormatter.format(t))}
          </animated.p>
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
