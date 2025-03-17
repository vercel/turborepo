/* eslint-disable rulesdir/global-css */
import { VercelToolbar } from "@vercel/toolbar/next";
import { GeistSans } from "geist/font/sans";
import { GeistMono } from "geist/font/mono";
import type { ReactNode } from "react";
import type { Metadata } from "next";
import { Footer } from "@/components/nav/footer";
import { RootProvider } from "@/components/root-provider";
import { PRODUCT_SLOGANS } from "@/lib/constants";
import { createMetadata } from "@/lib/create-metadata";
import { VercelTrackers } from "@/components/analytics";
import "./global.css";

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
      <body>
        <RootProvider>{children}</RootProvider>
        {shouldInjectToolbar ? <VercelToolbar /> : null}
        <Footer />
        <VercelTrackers />
      </body>
    </html>
  );
}
