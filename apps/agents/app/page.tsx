"use client";

import { useState } from "react";
import Link from "next/link";
import { triggerAudit } from "./actions";

export default function Home() {
  const [auditStatus, setAuditStatus] = useState<
    "idle" | "running" | "done" | "error"
  >("idle");

  async function handleRunAudit() {
    setAuditStatus("running");
    try {
      await triggerAudit();
      setAuditStatus("done");
    } catch {
      setAuditStatus("error");
    }
  }

  return (
    <main className="mx-auto max-w-2xl px-6 py-16 font-mono">
      <h1 className="mb-2 text-2xl font-bold">Turborepo Agents</h1>
      <p className="mb-8 text-neutral-500">
        Internal automation for the Turborepo repository.
      </p>

      <section className="mb-12">
        <h2 className="mb-4 text-lg font-semibold">Actions</h2>
        <div className="rounded border border-neutral-800 p-4">
          <div className="flex items-center justify-between">
            <div>
              <p className="font-medium">Security Audit</p>
              <p className="text-sm text-neutral-500">
                Run cargo audit + pnpm audit, fix vulnerabilities, post results
                to Slack.
              </p>
            </div>
            <button
              onClick={handleRunAudit}
              disabled={auditStatus === "running"}
              className="rounded bg-white px-4 py-2 text-sm font-medium text-black hover:bg-neutral-200 disabled:cursor-not-allowed disabled:opacity-50"
            >
              {auditStatus === "running" ? "Running..." : "Run audit"}
            </button>
          </div>
          {auditStatus === "done" && (
            <p className="mt-3 text-sm text-green-500">
              Audit triggered. Check Slack for progress.
            </p>
          )}
          {auditStatus === "error" && (
            <p className="mt-3 text-sm text-red-500">
              Failed to trigger audit. Check the logs.
            </p>
          )}
        </div>
      </section>

      <section>
        <h2 className="mb-4 text-lg font-semibold">History</h2>
        <Link
          href="/vuln-diffs"
          className="inline-block rounded border border-neutral-800 px-4 py-2 text-sm hover:bg-neutral-800"
        >
          View saved diffs
        </Link>
      </section>
    </main>
  );
}
