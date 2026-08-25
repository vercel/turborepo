import Link from "next/link";

import { WorkspaceList } from "./workspace-list";

const AGENT_RUNS_URL =
  "https://vercel.com/vercel-internal-apps/turborepo-factory/observability/agent-runs";

export default function WorkspacesPage() {
  return (
    <main
      className="mx-auto w-[min(960px,calc(100%_-_48px))] py-12 max-[720px]:w-[min(960px,calc(100%_-_32px))]"
      id="main-content"
    >
      <header className="flex items-end justify-between gap-6 max-[520px]:items-start max-[520px]:flex-col">
        <div>
          <h1 className="text-pretty text-[clamp(1.5rem,3vw,2rem)] leading-tight font-semibold tracking-[-0.04em]">
            Workspaces
          </h1>
          <p className="mt-2 text-sm text-muted-foreground">
            Durable Factory conversations and their sandboxes.
          </p>
        </div>
        <div className="flex items-center gap-4">
          <Link
            className="text-sm font-medium hover:underline hover:underline-offset-4"
            href="/work"
            prefetch={false}
          >
            Start work
          </Link>
          <a
            className="text-sm font-medium hover:underline hover:underline-offset-4"
            href={AGENT_RUNS_URL}
            rel="noreferrer"
            target="_blank"
          >
            Agent Runs <span className="sr-only">in a new tab</span>
            <span aria-hidden="true"> ↗</span>
          </a>
        </div>
      </header>
      <section className="mt-8" aria-label="Factory workspaces">
        <WorkspaceList />
      </section>
    </main>
  );
}
