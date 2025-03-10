import type { ReactNode } from "react";
import { HomeLayout as FumaLayout } from "fumadocs-ui/layouts/home";
import { navLinks } from "@/lib/nav-links";
import { FaviconHandler } from "../_components/favicon-handler";
import { TitleLogos } from "@/components/TitleLogos";

export default function Layout({
  children,
}: {
  children: ReactNode;
}): JSX.Element {
  return (
    <>
      <FaviconHandler />
      <FumaLayout
        githubUrl="https://github.com/vercel/turborepo"
        links={navLinks}
        nav={{
          title: <TitleLogos />,
        }}
      >
        {children}
      </FumaLayout>
    </>
  );
}
