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
  translations,
  content: [{ id: "docs", label: "Docs", dir: "content/docs", route: "/docs" }],
  ai: {
    prompt,
    suggestions
  }
});
