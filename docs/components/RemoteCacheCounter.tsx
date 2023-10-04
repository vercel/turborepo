import cn from "classnames";
import { useState, useEffect } from "react";
import { animated, useSpring, config } from "@react-spring/web";
import useTurborepoMinutesSaved from "../lib/useTurborepoMinutesSaved";
import Link from "next/link";

const counterFormatter = Intl.NumberFormat(undefined, {
  minimumIntegerDigits: 7,
  maximumFractionDigits: 0,
});

export default function RemoteCacheCounter() {
  const [targetMinutes, setTargetMinutes] = useState(0);
  const timeSaved = useTurborepoMinutesSaved();
  useEffect(() => {
    if (timeSaved) {
      setTargetMinutes(
        timeSaved.local_cache_minutes_saved +
          timeSaved.remote_cache_minutes_saved
      );
    }
  }, [timeSaved]);

  const spring = useSpring({
    from: { minutesSaved: 0 },
    minutesSaved: targetMinutes,
    config: config.molasses,
  });

  return (
    <Link
      href="/repo/docs/core-concepts/remote-caching"
      className="group mt-4 rounded-lg border border-transparent overflow-hidden bg-origin-border bg-gradient-to-r from-red-500 to-blue-500 dark:text-[#9ca3af] text-[#6b7280]"
    >
      <div className="p-4 dark:bg-[#111111] bg-white">
        <animated.p className="inline-block text-xl bg-gradient-to-r from-red-500 to-blue-500 bg-clip-text text-transparent tabular-nums">
          {spring.minutesSaved.to((t) => counterFormatter.format(t))}
        </animated.p>
        <div className="text-xs">Total Compute Minutes Saved</div>

        <div className="text-xs mt-4 group-hover:underline">
          Get Started With Remote Caching →
        </div>
      </div>
    </Link>
  );
}
