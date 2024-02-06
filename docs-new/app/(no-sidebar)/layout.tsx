import type { ReactNode } from "react";
import { Layout as FumaLayout } from "fumadocs-ui/layout";
import { NavbarChildren } from "@/app/_components/title";
import { DiscordLogo, GithubLogo } from "@/app/_components/logos";

export default function Layout({ children }: { children: ReactNode }) {
  return (
    <>
      <FumaLayout
        nav={{
          title: <></>,
          children: <NavbarChildren />,
          links: [
            {
              href: "https://github.com/vercel/turbo",
              label: "GitHub",
              icon: <GithubLogo />,
            },
            {
              href: "https://example.com",
              label: "Example",
              icon: <DiscordLogo />,
            },
          ],
        }}
      >
        {children}
      </FumaLayout>
    </>
  );
}
