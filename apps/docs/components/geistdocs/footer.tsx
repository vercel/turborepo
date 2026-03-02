import { SiVercel } from "@icons-pack/react-simple-icons";
import { GitHubButton } from "./github-button";
import { LanguageSelector } from "./language-selector";
import { RSSButton } from "./rss-button";
import { ThemeToggle } from "./theme-toggle";

interface FooterProps {
  copyright?: string;
}

export const Footer = ({
  copyright = `Copyright Vercel ${new Date().getFullYear()}. All rights reserved.`,
}: FooterProps) => (
  <footer className="border-t px-4 py-5 md:px-6">
    <div className="mx-auto flex flex-col items-center justify-between gap-4 sm:flex-row">
      <div className="flex items-center gap-2">
        <SiVercel className="size-4 shrink-0" />
        <p className="text-center text-muted-foreground text-sm sm:text-left">
          {copyright}
        </p>
      </div>
      <div className="flex items-center gap-2">
        <LanguageSelector />
        <RSSButton />
        <GitHubButton />
        <ThemeToggle />
      </div>
    </div>
  </footer>
);
