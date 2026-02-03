import {
  TurborepoLogo,
  TurborepoWordmarkDark,
  TurborepoWordmarkLight
} from "@/components/logos";

export const Logo = () => (
  <>
    {/* Logo icon only on screens <= 400px and between 768px-940px */}
    <TurborepoLogo className="block h-6 w-auto min-[401px]:hidden min-[768px]:block min-[941px]:hidden" />
    {/* Wordmark on screens 401px-767px and > 940px */}
    <TurborepoWordmarkDark className="hidden h-6 w-auto dark:min-[401px]:block dark:min-[768px]:hidden dark:min-[941px]:block" />
    <TurborepoWordmarkLight className="hidden h-6 w-auto min-[401px]:block dark:min-[401px]:hidden min-[768px]:hidden min-[941px]:block dark:min-[941px]:hidden" />
  </>
);

export const github = {
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
