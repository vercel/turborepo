"use client";

import { motion, useSpring, useTransform } from "motion/react";
import { useEffect, useState } from "react";
import { cn } from "@/lib/utils";
import { useTurborepoMinutesSaved } from "./use-turborepo-minutes-saved";

const counterFormatter = Intl.NumberFormat(undefined, {
  maximumFractionDigits: 0
});

// A number to start the counter at that is lower than the actual time saved
// to make the counter not start at 0
const ARBITRARY_START_NUMBER = 540070107 / 60;

export function RemoteCacheCounterClient({
  className
}: {
  className?: string;
}) {
  const timeSaved = useTurborepoMinutesSaved()?.total;
  const [displayValue] = useState(ARBITRARY_START_NUMBER);

  const targetValue = timeSaved ? timeSaved / 60 : ARBITRARY_START_NUMBER;

  const springValue = useSpring(displayValue, {
    mass: 1,
    stiffness: 170,
    damping: 60
  });

  const display = useTransform(springValue, (val) =>
    counterFormatter.format(val)
  );

  useEffect(() => {
    springValue.set(targetValue);
  }, [targetValue, springValue]);

  return (
    <motion.p
      className={cn("inline-block tabular-nums min-w-[94.6875px]", className)}
    >
      {display}
    </motion.p>
  );
}
