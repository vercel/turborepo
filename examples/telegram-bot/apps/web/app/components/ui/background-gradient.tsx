"use client";
import React from "react";
import { cn } from "../../lib/utils";

interface BackgroundGradientProps {
  children: React.ReactNode;
  className?: string;
  containerClassName?: string;
}

export const BackgroundGradient = ({
  children,
  className,
  containerClassName,
}: BackgroundGradientProps) => {
  return (
    <div className={cn("relative p-[4px] group", containerClassName)}>
      <div
        className={cn(
          "absolute inset-0 rounded-3xl z-[1] opacity-60 group-hover:opacity-100 blur-xl transition duration-500 will-change-transform",
          " bg-[radial-gradient(circle_farthest-side_at_0_100%,#48b4e8,transparent),radial-gradient(circle_farthest-side_at_100%_0,#24a1de,transparent),radial-gradient(circle_farthest-side_at_100%_100%,#137cb6,transparent),radial-gradient(circle_farthest-side_at_0_0,#106394,#0d2d44)]"
        )}
      />
      <div className={cn("relative bg-slate-900 rounded-3xl z-10", className)}>
        {children}
      </div>
    </div>
  );
};
