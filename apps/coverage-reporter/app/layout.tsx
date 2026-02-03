import type { Metadata } from "next";
import Link from "next/link";
import "./globals.css";

export const metadata: Metadata = {
  title: "Turborepo Coverage",
  description: "Test coverage tracking for Turborepo"
};

export default function RootLayout({
  children
}: {
  children: React.ReactNode;
}) {
  return (
    <html lang="en">
      <body>
        <nav>
          <ul>
            <li>
              <Link href="/">Dashboard</Link>
            </li>
            <li>
              <Link href="/crates">Crates</Link>
            </li>
          </ul>
        </nav>
        {children}
      </body>
    </html>
  );
}
