"use client";

import React from "react";
import { cn } from "@/lib/utils";

export function ChartWrapper({
  content: Chart,
  className
}: {
  content: React.ComponentType;
  className?: string;
  title?: string;
}) {
  return (
    <div className={cn("w-full h-[300px]", className)}>
      <Chart />
    </div>
  );
}
