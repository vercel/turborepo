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
    "Observe and operate Turborepo agent runs across Eve, Harness, and Vercel Sandbox.",
  title: "Turborepo Agent Control Plane"
};

interface RootLayoutProps {
  readonly children: ReactNode;
}

export default function RootLayout({ children }: RootLayoutProps) {
  return (
    <html lang="en">
      <body className={`${geistSans.variable} ${geistMono.variable}`}>
        <a className="skipLink" href="#main-content">
          Skip to content
        </a>
        <div className="appShell">
          <Navigation />
          {children}
        </div>
      </body>
    </html>
  );
}
