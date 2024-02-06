import "./global.css";
import { RootProvider } from "fumadocs-ui/provider";
import { Inter } from "next/font/google";
import type { ReactNode } from "react";
import { Layout as FumaLayout } from "fumadocs-ui/layout";
import { Footer } from "@/app/_components/footer";
import { LogoContext } from "@/app/_components/logo-context";
import { TurboAnimated } from "@/app/_components/logos/TurboAnimated";
import { SiteSwitcher } from "@/app/_components/site-switcher";
import Link from "next/link";
import { Title } from "@/app/_components/title";

const inter = Inter({
  subsets: ["latin"],
});

export default function Layout({ children }: { children: ReactNode }) {
  return (
    <html lang="en" className={inter.className}>
      <body>
        <RootProvider>
          <FumaLayout
            nav={{
              title: <></>,
              children: <Title />,
              githubUrl: "https://github.com/vercel/turbo",
            }}
          >
            {children}
          </FumaLayout>
        </RootProvider>
        <Footer />
      </body>
    </html>
  );
}
