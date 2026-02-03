import type { JSX, ReactNode } from "react";
import Link from "next/link";
import { TurborepoLogo } from "@/components/logos";
import { github, footerLinks } from "@/geistdocs";

const gitHubRepoUrl = `https://github.com/${github.owner}/${github.repo}`;
const communityUrl =
  footerLinks.community.find((l) => l.label === "Community")?.href ??
  "https://community.vercel.com/tag/turborepo";

export function NotFoundTemplate({
  content
}: {
  content?: ReactNode;
}): JSX.Element {
  return (
    <main className="relative flex h-full w-full flex-col overflow-hidden pt-36 pb-20">
      <div className="flex justify-center pb-24">
        <TurborepoLogo className="size-24" />
      </div>
      <h1 className="pb-4 text-center text-2xl font-bold">404</h1>
      {content ? (
        content
      ) : (
        <div className="mx-auto">
          <p className="text-muted-foreground">
            We couldn&apos;t find that link.
          </p>
          <ul className="mt-8 flex flex-col gap-y-3 text-sm">
            <li className="transition duration-100 text-muted-foreground hover:text-foreground">
              <Link href="/">Home</Link>
            </li>
            <li className="transition duration-100 text-muted-foreground hover:text-foreground">
              <Link href="/docs">Documentation</Link>
            </li>
            <li className="transition duration-100 text-muted-foreground hover:text-foreground">
              <a href={gitHubRepoUrl} rel="noopener" target="_blank">
                GitHub
              </a>
            </li>
            <li className="transition duration-100 text-muted-foreground hover:text-foreground">
              <a href={communityUrl} rel="noopener" target="_blank">
                Community
              </a>
            </li>
            <li className="transition duration-100 text-muted-foreground hover:text-foreground">
              <Link href="/sitemap.md">Sitemap for agents</Link>
            </li>
          </ul>
        </div>
      )}
    </main>
  );
}
