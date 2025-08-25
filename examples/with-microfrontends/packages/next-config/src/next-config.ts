import type { NextConfig } from "next";
import { withMicrofrontends } from "@vercel/microfrontends/next/config";

export const sharedNextConfig = (config?: NextConfig) =>
  withMicrofrontends(config ?? {});
