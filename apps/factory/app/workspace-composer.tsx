"use client";

import { useRouter } from "next/navigation";
import { useState } from "react";

import type { GatewayModel } from "../agent/lib/gateway-models";
import { Button } from "../components/ui/button";
import type { PublicWorkspace } from "./workspace-types";

interface WorkspaceComposerProps {
  readonly defaultModel: string;
  readonly models: readonly GatewayModel[];
}

export function WorkspaceComposer({
  defaultModel,
  models
}: WorkspaceComposerProps) {
  const router = useRouter();
  const [title, setTitle] = useState("");
  const [prompt, setPrompt] = useState("");
  const [model, setModel] = useState(defaultModel);
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function createWorkspace() {
    const message = prompt.trim();
    if (!message || submitting) return;
    setSubmitting(true);
    setError(null);
    try {
      const response = await fetch("/eve/v1/workspaces", {
        method: "POST",
        headers: {
          "content-type": "application/json",
          "x-operator-action": "create-workspace"
        },
        body: JSON.stringify({
          ...(title.trim() ? { title: title.trim() } : {}),
          model,
          prompt: message
        })
      });
      if (!response.ok) {
        const body = (await response.json().catch(() => ({}))) as {
          error?: string;
        };
        throw new Error(
          body.error ?? `Could not create workspace (${response.status}).`
        );
      }
      const workspace = (await response.json()) as PublicWorkspace;
      router.push(`/workspaces/${encodeURIComponent(workspace.id)}`);
    } catch (cause) {
      setError(
        cause instanceof Error ? cause.message : "Could not create workspace."
      );
      setSubmitting(false);
    }
  }

  return (
    <form
      className="grid gap-4"
      onSubmit={(event) => {
        event.preventDefault();
        void createWorkspace();
      }}
    >
      <div className="grid gap-2">
        <label className="text-sm font-medium" htmlFor="workspace-title">
          Title{" "}
          <span className="font-normal text-muted-foreground">(optional)</span>
        </label>
        <input
          className="min-h-10 rounded-md border border-input bg-background px-3 text-sm focus-visible:outline-2 focus-visible:outline-ring focus-visible:outline-offset-2"
          disabled={submitting}
          id="workspace-title"
          onChange={(event) => setTitle(event.target.value)}
          placeholder="Fix affected package detection"
          value={title}
        />
      </div>
      <div className="grid gap-2">
        <label className="text-sm font-medium" htmlFor="workspace-model">
          Model
        </label>
        <select
          className="min-h-10 rounded-md border border-input bg-background px-3 text-sm focus-visible:outline-2 focus-visible:outline-ring focus-visible:outline-offset-2"
          disabled={submitting}
          id="workspace-model"
          onChange={(event) => setModel(event.target.value)}
          value={model}
        >
          {models.map((option) => (
            <option key={option.id} value={option.id}>
              {option.name} ({option.ownedBy})
            </option>
          ))}
        </select>
      </div>
      <div className="grid gap-2">
        <label className="text-sm font-medium" htmlFor="workspace-prompt">
          What should Factory do?
        </label>
        <textarea
          className="min-h-32 resize-y rounded-md border border-input bg-background p-3 text-sm focus-visible:outline-2 focus-visible:outline-ring focus-visible:outline-offset-2"
          disabled={submitting}
          id="workspace-prompt"
          onChange={(event) => setPrompt(event.target.value)}
          placeholder="Investigate the affected-glob warning and implement the smallest safe fix."
          required
          value={prompt}
        />
      </div>
      {error ? (
        <p className="text-sm text-destructive" role="alert">
          {error}
        </p>
      ) : null}
      <Button
        className="justify-self-start"
        disabled={submitting || !prompt.trim()}
        type="submit"
      >
        {submitting ? "Creating workspace…" : "Create workspace"}
      </Button>
    </form>
  );
}
