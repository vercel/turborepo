"use client";

import { useTween } from "phase/react";
import { cn } from "@/lib/utils";
import { REMOTE_CACHE_COUNTER_START_HOURS } from "./constants";
import { useTurborepoMinutesSaved } from "./use-turborepo-minutes-saved";

const counterFormatter = Intl.NumberFormat(undefined, {
  maximumFractionDigits: 0,
});

// A number to start the counter at that is lower than the actual time saved
// to make the counter not start at 0
const ARBITRARY_START_NUMBER = REMOTE_CACHE_COUNTER_START_HOURS;

export function RemoteCacheCounterClient({
  className,
}: {
  className?: string;
}) {
  const timeSaved = useTurborepoMinutesSaved()?.total;
  const targetValue = timeSaved ? timeSaved / 60 : ARBITRARY_START_NUMBER;
  const displayValue = useTween({ target: targetValue, duration: 1200 });

  return (
    <p className={cn("inline-block tabular-nums min-w-[94.6875px]", className)}>
      {counterFormatter.format(displayValue)}
    </p>
  );
}
