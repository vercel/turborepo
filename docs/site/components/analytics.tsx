"use client";
import { Analytics } from "@vercel/analytics/next";
import { SpeedInsights } from "@vercel/speed-insights/next";
import type { JSX } from "react";

export function VercelTrackers(): JSX.Element {
  return (
    <>
      <Analytics />
      <SpeedInsights />
    </>
  );
}
