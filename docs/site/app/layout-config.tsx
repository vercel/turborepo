import type { BaseLayoutProps } from "fumadocs-ui/layouts/shared";
import { Navigation } from "#components/nav/index.tsx";

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
