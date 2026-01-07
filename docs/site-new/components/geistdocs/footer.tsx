import Link from "next/link";
import { SiVercel } from "@icons-pack/react-simple-icons";
import { footerLinks } from "@/geistdocs";
import { ThemeToggle } from "./theme-toggle";

type FooterLink = {
  href: string;
  label: string;
};

const FooterSection = ({
  title,
  links
}: {
  title: string;
  links: FooterLink[];
}) => (
  <div className="flex flex-col gap-y-3 text-sm">
    <h4 className="font-medium">{title}</h4>
    <ul className="flex flex-col gap-y-3 text-muted-foreground">
      {links.map((link) => (
        <li
          key={link.href}
          className="transition duration-100 hover:text-foreground"
        >
          {link.href.startsWith("http") ? (
            <a href={link.href} rel="noopener" target="_blank">
              {link.label}
            </a>
          ) : (
            <Link href={link.href}>{link.label}</Link>
          )}
        </li>
      ))}
    </ul>
  </div>
);

export const Footer = () => (
  <footer className="w-full border-t py-12">
    <div className="mx-auto flex w-full max-w-screen-xl flex-col gap-y-12 px-6">
      <div className="flex w-full flex-col items-start justify-between gap-y-12 md:flex-row">
        <SiVercel className="size-6" />
        <div className="grid grid-cols-2 gap-x-16 gap-y-12 md:auto-cols-max md:grid-flow-col md:gap-x-24">
          <FooterSection title="Resources" links={footerLinks.resources} />
          <FooterSection title="Community" links={footerLinks.community} />
          <FooterSection title="Vercel" links={footerLinks.company} />
          <FooterSection title="Legal" links={footerLinks.legal} />
        </div>
      </div>
      <div className="md:ml-auto">
        <ThemeToggle />
      </div>
    </div>
  </footer>
);
