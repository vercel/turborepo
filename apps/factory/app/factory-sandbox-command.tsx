"use client";

import { useState } from "react";

import { Button } from "../components/ui/button";
import { factorySandboxCommand } from "../lib/factory-sandbox-command";

export function FactorySandboxCommand({
  snapshotId
}: {
  readonly snapshotId: string | null;
}) {
  const command = snapshotId ? factorySandboxCommand(snapshotId) : null;
  const [copiedCommand, setCopiedCommand] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  async function copy() {
    if (!command) return;
    try {
      await navigator.clipboard.writeText(command);
      setCopiedCommand(command);
      setError(null);
    } catch {
      setError("Could not copy. Select the command and copy it manually.");
    }
  }

  return (
    <section
      className="rounded-md border border-border p-4"
      aria-label="Open a sandbox from the factory image"
    >
      <div className="flex flex-wrap items-center justify-between gap-3">
        <h3 className="text-sm font-medium">Hop into a sandbox</h3>
        <Button
          disabled={!command}
          onClick={() => void copy()}
          size="sm"
          type="button"
          variant="outline"
        >
          {command && copiedCommand === command ? "Copied" : "Copy command"}
        </Button>
      </div>
      <p className="mt-2 text-sm text-muted-foreground">
        Start a fresh sandbox from the same published snapshot Factory uses. Run
        with an authenticated Vercel Sandbox CLI in the Factory project, a local
        GitHub CLI login, and <code>AI_GATEWAY_API_KEY</code> set locally.
      </p>
      {command ? (
        <pre className="mt-3 overflow-x-auto rounded-md bg-secondary p-3 font-mono text-xs">
          <code>{command}</code>
        </pre>
      ) : (
        <p className="mt-3 text-sm text-muted-foreground">
          No factory image has been published yet.
        </p>
      )}
      <p className="mt-3 text-xs text-muted-foreground">
        Once connected, run <code>cd turborepo</code>. The sandbox stops after
        45 minutes.
      </p>
      <p className="mt-2 text-xs text-muted-foreground">
        Credentials resolve locally when you run the command, not on this page
        or in the snapshot. Expanded values may be visible to local process
        inspection.
      </p>
      <p className="mt-2 text-xs text-muted-foreground" role="status">
        {error ??
          (command && copiedCommand === command ? "Command copied." : "")}
      </p>
    </section>
  );
}
