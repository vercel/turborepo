import { RootProvider } from "fumadocs-ui/provider";
import { VercelToolbar } from "@vercel/toolbar/next";
import { GeistSans } from "geist/font/sans";
import { GeistMono } from "geist/font/mono";
import type { ReactNode } from "react";
import { Footer } from "@/app/_components/footer";
import "./global.css";

export default function Layout({ children }: { children: ReactNode }) {
  const shouldInjectToolbar = process.env.NODE_ENV === "development";

  return (
    <html className={`${GeistSans.variable} ${GeistMono.variable}`} lang="en">
      <body>
        <RootProvider>{children}</RootProvider>
        {shouldInjectToolbar && <VercelToolbar />}
        <Footer />
      </body>
    </html>
  );
}
