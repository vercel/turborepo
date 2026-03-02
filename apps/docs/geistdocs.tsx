import { BookHeartIcon } from "lucide-react";

export const Logo = () => (
  <div className="flex items-center gap-2">
    <BookHeartIcon className="size-5" />
    <p className="font-semibold text-xl tracking-tight">Geistdocs</p>
  </div>
);

export const github = {
  owner: "vercel",
  repo: "geistdocs",
};

export const nav = [
  {
    label: "Docs",
    href: "/docs",
  },
  {
    label: "Source",
    href: `https://github.com/${github.owner}/${github.repo}/`,
  },
];

export const suggestions = [
  "What is Geistdocs?",
  "What can I make with Geistdocs?",
  "What syntax does Geistdocs support?",
  "How do I deploy my Geistdocs site?",
];

export const title = "Geistdocs Documentation";

export const prompt =
  "You are a helpful assistant specializing in answering questions about Geistdocs, a modern documentation template built with Next.js and Fumadocs.";

export const translations = {
  en: {
    displayName: "English",
  },
  cn: {
    displayName: "Chinese",
    search: "搜尋文檔",
  },
};

export const basePath: string | undefined = undefined;

/**
 * Unique identifier for this site, used in markdown request tracking analytics.
 * Each site using geistdocs should set this to a unique value (e.g. "ai-sdk-docs", "next-docs").
 */
export const siteId: string | undefined = undefined;
