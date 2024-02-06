import type { ReactNode } from "react";
import { DocsLayout } from "fumadocs-ui/layout";
import { NavbarChildren } from "@/app/_components/title";
import { DiscordLogo, GithubLogo } from "@/app/_components/logos";
import { pageTree } from "@/app/source";

export default function Layout({ children }: { children: ReactNode }) {
  return (
    <>
      <DocsLayout
        tree={pageTree}
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
      </DocsLayout>
    </>
  );
}
