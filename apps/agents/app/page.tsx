const ENDPOINTS = [
  {
    method: "POST",
    path: "/api/github",
    description:
      "GitHub webhook. Analyzes new issues for missing reproductions, notifies Slack only when one is needed."
  },
  {
    method: "POST",
    path: "/api/slack/actions",
    description:
      "Slack interactive actions. Handles approval buttons for reproduction requests, audit fix review, and PR creation."
  },
  {
    method: "GET",
    path: "/api/cron/audit",
    description:
      "Security audit cron. Runs cargo audit + pnpm audit, posts results to Slack with a button to launch the fix agent."
  }
];

export default function Home() {
  return (
    <main className="mx-auto max-w-2xl px-6 py-16 font-mono">
      <h1 className="mb-2 text-2xl font-bold">Turborepo Agents</h1>
      <p className="mb-8 text-neutral-500">
        Internal automation for the Turborepo repository.
      </p>

      <section>
        <h2 className="mb-4 text-lg font-semibold">Endpoints</h2>
        <ul className="space-y-4">
          {ENDPOINTS.map((ep) => (
            <li key={ep.path} className="rounded border border-neutral-800 p-4">
              <div className="mb-1 flex items-center gap-2">
                <span className="rounded bg-neutral-800 px-2 py-0.5 text-xs text-neutral-300">
                  {ep.method}
                </span>
                <code className="text-sm">{ep.path}</code>
              </div>
              <p className="text-sm text-neutral-500">{ep.description}</p>
            </li>
          ))}
        </ul>
      </section>
    </main>
  );
}
