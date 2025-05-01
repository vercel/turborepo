import Fumalink from "fumadocs-core/link";
import { VercelLogo } from "#app/_components/logos.tsx";
import { ThemeSwitcher } from "./theme-switcher/index";

interface FooterItem {
  href: string;
  label: string;
}

const FOOTER_ITEMS = {
  legal: [
    { href: "https://vercel.com/legal/privacy-policy", label: "Privacy" },
    { href: "/terms", label: "Terms" },
    { href: "/governance", label: "Governance" },
    { href: "/docs/telemetry", label: "Telemetry" },
  ],
  resources: [
    { href: "/blog", label: "Blog" },
    { href: "https://github.com/vercel/turborepo/releases", label: "Releases" },
    { href: "/docs", label: "Docs" },
  ],
  company: [
    { href: "https://vercel.com/about", label: "About" },
    { href: "https://vercel.com/oss", label: "Open source" },
  ],
  community: [
    { href: "https://github.com/vercel/turborepo", label: "GitHub" },
    { href: "https://community.vercel.com/tag/turborepo", label: "Community" },
    { href: "https://bsky.app/profile/turborepo.com", label: "Bluesky" },
    { href: "https://x.com/turborepo", label: "X" },
  ],
};

const NavItems = ({ category }: { category: Array<FooterItem> }) => {
  return (
    <ul className="flex flex-col gap-y-3 text-gray-900">
      {category.map((item) => {
        return (
          <li
            key={item.href}
            className="transition duration-100 hover:text-gray-1000"
          >
            <Fumalink href={item.href}>{item.label}</Fumalink>
          </li>
        );
      })}
    </ul>
  );
};

export const Footer = () => {
  return (
    <footer className="w-full border-t border-gray-200 bg-background-100 py-12">
      <div className="mx-auto flex w-full max-w-screen-xl flex-col gap-y-12 px-6">
        <div className="flex w-full flex-col items-start justify-between gap-y-12 md:flex-row">
          <VercelLogo className="h-6" />
          <div className="grid grid-cols-[auto_1fr] gap-x-36 gap-y-12 md:auto-cols-max md:grid-flow-col md:gap-x-24">
            <div className="flex flex-col gap-y-3 text-sm">
              <h4 className="font-medium">Resources</h4>
              <NavItems category={FOOTER_ITEMS.resources} />
            </div>
            <div className="flex flex-col gap-y-3 text-sm">
              <h4 className="font-medium">Community</h4>
              <NavItems category={FOOTER_ITEMS.community} />
            </div>

            <div className="flex flex-col gap-y-3 text-sm">
              <h4 className="font-medium">Vercel</h4>
              <NavItems category={FOOTER_ITEMS.company} />
            </div>
            <div className="flex flex-col gap-y-3 text-sm">
              <h4 className="font-medium">Legal</h4>
              <NavItems category={FOOTER_ITEMS.legal} />
            </div>
          </div>
        </div>
        <ThemeSwitcher className="md:ml-auto" />
      </div>
    </footer>
  );
};
