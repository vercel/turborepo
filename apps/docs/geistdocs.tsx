import type { GeistdocsAgentReadinessConfig } from "@vercel/geistdocs/config";
import {
  TurborepoLogo,
  TurborepoWordmarkDark,
  TurborepoWordmarkLight
} from "@/components/logos";

export const Logo = () => (
  <>
    {/* Logo icon only on screens <= 480px and between 768px-940px */}
    <TurborepoLogo className="block h-6 w-auto min-[481px]:hidden min-[768px]:block min-[941px]:hidden" />
    {/* Wordmark on screens 481px-767px and > 940px */}
    <TurborepoWordmarkDark className="hidden h-6 w-auto dark:min-[481px]:block dark:min-[768px]:hidden dark:min-[941px]:block" />
    <TurborepoWordmarkLight className="hidden h-6 w-auto min-[481px]:block dark:min-[481px]:hidden min-[768px]:hidden min-[941px]:block dark:min-[941px]:hidden" />
  </>
);

export const github = {
  branch: "main",
  editPath: "apps/docs/content/docs/{path}",
  owner: "vercel",
  repo: "turborepo"
};

export const nav = [
  {
    label: "Docs",
    href: "/docs"
  },
  {
    label: "Blog",
    href: "/blog"
  },
  {
    label: "Showcase",
    href: "/showcase"
  },
  {
    label: "Enterprise",
    href: "https://vercel.com/contact/sales?utm_source=turborepo.dev&utm_medium=referral&utm_campaign=header-enterpriseLink"
  }
];

export const footerLinks = {
  resources: [
    { href: "/blog", label: "Blog" },
    { href: "https://github.com/vercel/turborepo/releases", label: "Releases" },
    { href: "/docs", label: "Docs" }
  ],
  community: [
    { href: "https://github.com/vercel/turborepo", label: "GitHub" },
    { href: "https://community.vercel.com/tag/turborepo", label: "Community" },
    { href: "https://bsky.app/profile/turborepo.dev", label: "Bluesky" },
    { href: "https://x.com/turborepo", label: "X" }
  ],
  company: [
    { href: "https://vercel.com/about", label: "About" },
    { href: "https://vercel.com/oss", label: "Open source" }
  ],
  legal: [
    { href: "https://vercel.com/legal/privacy-policy", label: "Privacy" },
    { href: "/terms", label: "Terms" },
    { href: "/governance", label: "Governance" },
    { href: "/docs/telemetry", label: "Telemetry" },
    { href: "#", label: "Cookie Preferences", id: "fides-modal-link" }
  ]
};

export const suggestions = [
  "What is Turborepo?",
  "How do I set up a monorepo with Turborepo?",
  "What is Remote Caching?",
  "How do I configure tasks in turbo.json?"
];

export const title = "Turborepo Documentation";

export const prompt = `You are a helpful assistant specializing in answering questions about Turborepo, a high-performance build system for JavaScript and TypeScript monorepos. You help users understand caching, task configuration, and monorepo best practices. Be concise.`;

export const agent = {
  product: {
    name: "Turborepo",
    description:
      "Turborepo is a high-performance build system for JavaScript and TypeScript monorepos, written in Rust. It provides incremental task running, local and remote caching, and parallel execution.",
    category: "Build system",
    audience: [
      "JavaScript and TypeScript developers",
      "Monorepo maintainers",
      "Platform and developer experience teams"
    ],
    useCases: [
      "Run tasks across a monorepo with caching and parallelism",
      "Share build caches locally and remotely (Remote Caching)",
      "Configure task pipelines in turbo.json",
      "Prune monorepos for lightweight deploys"
    ]
  },
  links: [
    {
      label: "Turborepo source",
      href: `https://github.com/${github.owner}/${github.repo}`,
      description: "Source repository for Turborepo"
    },
    {
      label: "Releases",
      href: `https://github.com/${github.owner}/${github.repo}/releases`,
      description: "Turborepo release notes and changelogs"
    }
  ]
} satisfies GeistdocsAgentReadinessConfig;

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

export const siteId: string | undefined = "turborepo";
