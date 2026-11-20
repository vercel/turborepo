import { GeistMono } from "geist/font/mono";
import { GeistSans } from "geist/font/sans";
import type { Metadata } from "next";
import type { ReactNode } from "react";
import QueryProvider from "@/providers/query-provider";
import "./globals.css";

export const metadata: Metadata = {
  title: "Next.js + Elysia",
  description: "Turborepo example with Next.js and Elysia",
};

export default function RootLayout({
  children,
}: Readonly<{
  children: ReactNode;
}>) {
  return (
    <html
      className={`${GeistSans.variable} ${GeistMono.variable} h-full antialiased`}
      lang="en"
    >
      <body className="flex min-h-full flex-col">
        <QueryProvider>
          <div className="relative flex min-h-screen flex-col selection:bg-black selection:text-white">
            {children}
          </div>
        </QueryProvider>
      </body>
    </html>
  );
}
