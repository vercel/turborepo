import { RootProvider } from "fumadocs-ui/provider";
import { VercelToolbar } from "@vercel/toolbar/next";
import { Inter } from "next/font/google";
import type { ReactNode } from "react";
import { Footer } from "@/app/_components/footer";
import "./global.css";

const inter = Inter({
  subsets: ["latin"],
});

export default function Layout({ children }: { children: ReactNode }) {
  const shouldInjectToolbar = process.env.NODE_ENV === "development";

  return (
    <html className={inter.className} lang="en">
      <body>
        <RootProvider>{children}</RootProvider>
        {shouldInjectToolbar && <VercelToolbar />}
        <Footer />
      </body>
    </html>
  );
}
