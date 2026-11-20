"use client";

import { Analytics } from "@vercel/analytics/next";
import { GeistdocsProvider as PackageProvider } from "@vercel/geistdocs/layout";
import { SpeedInsights } from "@vercel/speed-insights/next";
import type { ComponentProps, ReactNode } from "react";
import { config } from "@/lib/geistdocs/config";

type GeistdocsProviderProps = Omit<
  ComponentProps<typeof PackageProvider>,
  "config"
> & {
  basePath: string | undefined;
  children?: ReactNode;
  className?: string;
  lang?: string;
};

export const GeistdocsProvider = ({
  basePath: _basePath,
  className: _className,
  lang,
  ...props
}: GeistdocsProviderProps) => {
  return (
    <>
      <PackageProvider config={config} lang={lang} {...props} />
      <Analytics />
      <SpeedInsights />
    </>
  );
};
