import type { Metadata } from "next";
import "./globals.css";

export const metadata: Metadata = {
  title: "Turborepo Agents",
  description: "Internal automation agents for the Turborepo repository"
};

export default function RootLayout({
  children
}: {
  children: React.ReactNode;
}) {
  return (
    <html lang="en">
      <body>{children}</body>
    </html>
  );
}
