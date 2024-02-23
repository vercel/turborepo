import { RootProvider } from "fumadocs-ui/provider";
import { Inter } from "next/font/google";
import type { ReactNode } from "react";
import { Footer } from "@/app/_components/footer";
import "./global.css";

const inter = Inter({
  subsets: ["latin"],
});

export default function Layout({ children }: { children: ReactNode }) {
  return (
    <html className={inter.className} lang="en">
      <body>
        <RootProvider>{children}</RootProvider>
        <Footer />
      </body>
    </html>
  );
}
