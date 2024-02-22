import React from "react";
import { Callout as FumaCallout } from "fumadocs-ui/components/callout";

export type FumaCalloutProps =
  | React.ComponentProps<typeof FumaCallout> & {
      type: React.ComponentProps<typeof FumaCallout>["type"] | "good-to-know";
    };

const THEMES = {
  info: {
    background: "bg-blue-100 dark:bg-blue-400 dark:bg-opacity-20",
    text: "text-blue-900/80 dark:text-blue-100/80",
    border: "border border-blue-400/40",
  },
  error: {
    background: "bg-red-200 dark:bg-red-400 dark:bg-opacity-20",
    text: "text-red-900/90 dark:text-red-100/80",
    border: "border border-red-400/40",
  },
  warn: {
    background: "bg-orange-100 dark:bg-orange-400 dark:bg-opacity-20",
    text: "text-orange-900/80 dark:text-orange-100/80",
    border: "border border-orange-400/40",
  },
  ["good-to-know"]: {
    background: "bg-transparent",
    text: "text-foreground",
    border: "border border-foreground/40",
  },
};

const iconStyles = "fill-background/80 w-6 h-6";

const ICONS = {
  info: (
    <svg
      className={`${THEMES.info.text} ${iconStyles}`}
      data-testid="geist-icon"
      height="14"
      shape-rendering="geometricPrecision"
      stroke="currentColor"
      stroke-linecap="round"
      stroke-linejoin="round"
      stroke-width="1.5"
      viewBox="0 0 24 24"
      width="14"
    >
      <circle cx="12" cy="12" r="10" fill="var(--geist-fill)" />
      <path d="M12 16v-4" stroke="var(--geist-stroke)" />
      <path d="M12 8h.01" stroke="var(--geist-stroke)" />
    </svg>
  ),
  error: (
    <svg
      className={`${THEMES.error.text} ${iconStyles}`}
      data-testid="geist-icon"
      fill="none"
      height="24"
      shapeRendering="geometricPrecision"
      stroke="currentColor"
      strokeLinecap="round"
      strokeLinejoin="round"
      strokeWidth="1.5"
      viewBox="0 0 24 24"
      width="24"
    >
      <path d="M7.86 2h8.28L22 7.86v8.28L16.14 22H7.86L2 16.14V7.86L7.86 2z" />
      <path d="M12 8v4" />
      <path d="M12 16h.01" />
    </svg>
  ),
  warn: (
    <svg
      className={`${THEMES.warn.text} ${iconStyles}`}
      data-testid="geist-icon"
      fill="none"
      height="24"
      shape-rendering="geometricPrecision"
      stroke="currentColor"
      stroke-linecap="round"
      stroke-linejoin="round"
      stroke-width="1.5"
      viewBox="0 0 24 24"
      width="24"
    >
      <path d="M10.29 3.86L1.82 18a2 2 0 001.71 3h16.94a2 2 0 001.71-3L13.71 3.86a2 2 0 00-3.42 0z" />
      <path d="M12 9v4" />
      <path d="M12 17h.01" />
    </svg>
  ),
  ["good-to-know"]: (
    <p className={`m-0 ${THEMES["good-to-know"].text} font-medium`}>
      Good to know:
    </p>
  ),
};

export function Callout({ type, ...props }: FumaCalloutProps) {
  return (
    <FumaCallout
      type={type}
      className={Object.values(THEMES[type || "info"]).join(" ") + " leading-6"}
      icon={ICONS[type || "info"]}
      {...props}
    />
  );
}
