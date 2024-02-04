import { Footer } from "@/app/_components/footer";
import "./global.css";
import { RootProvider } from "fumadocs-ui/provider";
import { Inter } from "next/font/google";
import type { ReactNode } from "react";
import { Header } from "@/app/_components/header";

const inter = Inter({
  subsets: ["latin"],
});

export default function Layout({ children }: { children: ReactNode }) {
  return (
    <html lang="en" className={inter.className}>
      <body>
        <Header />
        <RootProvider>{children}</RootProvider>
        <Footer />
      </body>
    </html>
  );
}
