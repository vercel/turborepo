import type { DocsLayoutProps } from "fumadocs-ui/layouts/docs";
import { navLinks } from "@/lib/nav-links";
import { TitleLogos } from "@/components/TitleLogos";
import { gitHubRepoUrl } from "@/lib/constants";

export const layoutPropsWithSidebar: Omit<
  DocsLayoutProps,
  "tree" | "children"
> = {
  links: navLinks,
  githubUrl: gitHubRepoUrl,
  nav: {
    title: <TitleLogos />,
  },
  sidebar: {
    defaultOpenLevel: 0,
    collapsible: true,
  },
};
