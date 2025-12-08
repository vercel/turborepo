"use client";

import { animated, useSpring } from "@react-spring/web";
import { useEffect, useState } from "react";
import { cn } from "#components/cn.ts";
import { useTurborepoMinutesSaved } from "./use-turborepo-minutes-saved";

const counterFormatter = Intl.NumberFormat(undefined, {
  maximumFractionDigits: 0,
});

// A number to start the counter at that is lower than the actual time saved
// to make the counter not start at 0
const ARBITRARY_START_NUMBER = 240000000 / 60;

export function RemoteCacheCounterClient({
  className,
}: {
  className?: string;
}): JSX.Element {
  const timeSaved = useTurborepoMinutesSaved()?.total;
  const [initialValue, setInitialValue] = useState<number | undefined>(
    timeSaved ? timeSaved / 60 : undefined
  );

  useEffect(() => {
    if (timeSaved) {
      setInitialValue(timeSaved / 60);
    }
  }, []);

  const dur = Number.isFinite(initialValue)
    ? initialValue
    : ARBITRARY_START_NUMBER;
  const spring = useSpring({
    val: timeSaved ? timeSaved / 60 : ARBITRARY_START_NUMBER,
    from: { val: dur },
    config: { mass: 1, tension: 170, friction: 60, clamp: true },
  });

  return (
    <animated.p
      className={cn("inline-block tabular-nums min-w-[94.6875px]", className)}
    >
      {spring.val?.to((val) => counterFormatter.format(val))}
    </animated.p>
  );
}
