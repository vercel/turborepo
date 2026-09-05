import { defineConfig } from "@vercel/geistdocs/config";
import {
  agent,
  basePath,
  github,
  Logo,
  nav,
  prompt,
  siteId,
  suggestions,
  title,
  translations
} from "@/geistdocs";
import { siteUrl } from "./site-url";

export const config = defineConfig({
  title,
  agent,
  defaultLanguage: "en",
  logo: <Logo />,
  github,
  nav,
  // Drops Turborepo (this site) from geistdocs' default OSS products menu.
  navbarActiveProduct: "turborepo",
  basePath,
  siteId,
  siteUrl: siteUrl.toString(),
  translations,
  content: [
    { id: "docs", label: "Docs", dir: "content/docs", route: "/docs" },
    {
      id: "openapi",
      label: "Remote Cache API",
      dir: "content/openapi",
      route: "/docs/openapi"
    }
  ],
  ai: {
    prompt,
    suggestions
  }
});
