import { VercelToolbar } from "@vercel/toolbar/next";
import { GeistSans } from "geist/font/sans";
import { GeistMono } from "geist/font/mono";
import type { ReactNode } from "react";
import type { Metadata } from "next";
import clsx from "clsx";
import { PRODUCT_SLOGANS } from "#lib/constants.ts";
import { createMetadata } from "#lib/create-metadata.ts";
import { VercelTrackers } from "#components/analytics.tsx";
import "./global.css";
import { RootProvider } from "#components/root-provider.tsx";
import { Footer } from "#components/nav/footer.tsx";
import { FaviconHandler } from "./_components/favicon-handler";

export function generateMetadata(): Metadata {
  return createMetadata({
    description: PRODUCT_SLOGANS.turbo,
    canonicalPath: "/",
  });
}

export default function Layout({
  children,
}: {
  children: ReactNode;
}): JSX.Element {
  const shouldInjectToolbar = process.env.NODE_ENV === "development";

  return (
    <html
      className={`${GeistSans.variable} ${GeistMono.variable}`}
      lang="en"
      suppressHydrationWarning
    >
      <body className={clsx("flex min-h-svh flex-col antialiased")}>
        <RootProvider>
          {children}
          <Footer />
        </RootProvider>
        <FaviconHandler />
        <VercelTrackers />
        {shouldInjectToolbar ? <VercelToolbar /> : null}
      </body>
    </html>
  );
}
