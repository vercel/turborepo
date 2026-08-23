import type { Metadata } from "next";
import { Geist, Geist_Mono } from "next/font/google";
import type { ReactNode } from "react";

import "./globals.css";
import { Navigation } from "./navigation";

const geistSans = Geist({
  subsets: ["latin"],
  variable: "--font-geist-sans"
});
const geistMono = Geist_Mono({
  subsets: ["latin"],
  variable: "--font-geist-mono"
});

export const metadata: Metadata = {
  description:
    "Observe and operate Turborepo agent runs across Eve, fx, and Vercel Sandbox.",
  title: "Turborepo Agent Control Plane"
};

interface RootLayoutProps {
  readonly children: ReactNode;
}

export default function RootLayout({ children }: RootLayoutProps) {
  return (
    <html className="min-w-80 bg-background" lang="en">
      <body
        className={`${geistSans.variable} ${geistMono.variable} min-h-screen bg-background font-sans text-foreground antialiased`}
      >
        <a
          className="fixed top-3 left-3 z-10 -translate-y-[200%] rounded-md bg-primary px-3 py-2 text-primary-foreground focus:translate-y-0"
          href="#main-content"
        >
          Skip to content
        </a>
        <div className="grid min-h-screen grid-cols-[240px_minmax(0,1fr)] max-[720px]:block">
          <Navigation />
          {children}
        </div>
      </body>
    </html>
  );
}
