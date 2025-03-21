import { Navigation } from "@/components/nav";
import type { BaseLayoutProps } from "fumadocs-ui/layouts/shared";

export const baseOptions: BaseLayoutProps = {
  nav: {
    component: <Navigation />,
    title: "Turborepo",
  },
  links: [
    {
      text: "Documentation",
      url: "/docs/introduction",
      active: "nested-url",
    },
  ],
};
