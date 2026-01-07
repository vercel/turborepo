import { BookHeartIcon } from "lucide-react";

export const Logo = () => (
  <div className="flex items-center gap-2">
    <BookHeartIcon className="size-5" />
    <p className="font-semibold text-xl tracking-tight">Geistdocs</p>
  </div>
);

export const github = {
  owner: undefined as string | undefined,
  repo: undefined as string | undefined
};

export const nav = [
  {
    label: "Docs",
    href: "/docs"
  },
  {
    label: "Source",
    href: `https://github.com/${github.owner}/${github.repo}/`
  }
];

export const suggestions = [
  "What is Vercel?",
  "What can I deploy with Vercel?",
  "What is Fluid Compute?",
  "How much does Vercel cost?"
];

export const title = "Geistdocs Documentation";

export const prompt =
  "You are a helpful assistant specializing in answering questions about Geistdocs, a modern documentation template built with Next.js and Fumadocs.";

export const translations = {
  en: {
    displayName: "English"
  },
  cn: {
    displayName: "Chinese",
    search: "搜尋文檔"
  }
};

export const basePath: string | undefined = undefined;
