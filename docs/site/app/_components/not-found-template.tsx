import Link from "next/link";
import type { ReactNode } from "react";
import { gitHubRepoUrl } from "#lib/constants.ts";
import { Turborepo } from "./turborepo";

export function NotFoundTemplate({
  content,
}: {
  content?: ReactNode;
}): JSX.Element {
  return (
    <main className="pt-36 pb-20 relative flex h-full w-full flex-col overflow-hidden [--geist-foreground:#fff] [--gradient-stop-1:0px] [--gradient-stop-2:120px] sm:[--gradient-stop-1:0px] sm:[--gradient-stop-2:120px] dark:[--geist-foreground:#000]">
      <div className="flex justify-center pb-24">
        <Turborepo />
      </div>
      <h1 className="text-2xl text-center font-bold pb-4">404</h1>
      {content ? (
        content
      ) : (
        <div className="prose mx-auto">
          <p>We couldn&apos;t find that link.</p>
          <ul>
            <li>
              <Link className="text-center" href="/">
                Home
              </Link>
            </li>

            <li>
              <Link className="text-center" href="/repo">
                Documentation
              </Link>
            </li>

            <li>
              <Link className="text-center" href={gitHubRepoUrl}>
                GitHub
              </Link>
            </li>

            <li>
              <Link
                className="text-center"
                href="https://community.vercel.com/tag/turborepo"
              >
                Community
              </Link>
            </li>
          </ul>
        </div>
      )}
    </main>
  );
}
