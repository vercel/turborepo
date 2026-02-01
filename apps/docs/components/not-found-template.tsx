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
        <div className="prose dark:prose-invert mx-auto">
          <p>We couldn&apos;t find that link.</p>
          <ul>
            <li>
              <Link className="text-center" href="/">
                Home
              </Link>
            </li>

            <li>
              <Link className="text-center" href="/docs">
                Documentation
              </Link>
            </li>

            <li>
              <Link className="text-center" href={gitHubRepoUrl}>
                GitHub
              </Link>
            </li>

            <li>
              <Link className="text-center" href={communityUrl}>
                Community
              </Link>
            </li>
          </ul>
        </div>
      )}
    </main>
  );
}
