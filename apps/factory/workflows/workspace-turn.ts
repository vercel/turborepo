import { randomUUID } from "node:crypto";

import type {
  HarnessAgentResumeSessionState,
  HarnessAgentSession
} from "@ai-sdk/harness/agent";

interface WorkspaceTurnInput {
  readonly turnId: string;
  readonly workspaceId: string;
}

const WORKSPACE_INSTRUCTIONS = `You are working interactively with a Turborepo maintainer in the vercel/turborepo checkout. Continue the existing conversation and preserve all work already in the checkout. Make the smallest correct change, verify it using the repository's own commands, and report concrete results. Do not open a pull request unless the maintainer asks. When asked, create an agents/<topic> branch, use a Conventional Commit title with an uppercase description and no scope, push it, and open a draft pull request with validation results in the body.`;

async function ensureWorkspaceSession(
  input: WorkspaceTurnInput
): Promise<void> {
  "use step";

  const { createHarnessAgent } = await import("../agent/lib/harness-agent");
  const { getWorkspace, mutateWorkspace } =
    await import("../agent/lib/workspace-store");
  const workspace = await getWorkspace(input.workspaceId);
  if (
    workspace === null ||
    workspace.activeTurnId !== input.turnId ||
    workspace.resumeState !== undefined
  )
    return;

  const agent = await createHarnessAgent(
    "opencode",
    "vercel",
    workspace.sessionId,
    { instructions: WORKSPACE_INSTRUCTIONS }
  );
  const session = await agent.createSession({ sessionId: workspace.sessionId });
  const resumeState = await session.detach();
  await mutateWorkspace(input.workspaceId, (current) =>
    current.activeTurnId === input.turnId && current.resumeState === undefined
      ? { ...current, resumeState }
      : current
  );
}

async function runWorkspaceTurn(input: WorkspaceTurnInput): Promise<void> {
  "use step";

  const { createHarnessAgent } = await import("../agent/lib/harness-agent");
  const { getWorkspace, mutateWorkspace } =
    await import("../agent/lib/workspace-store");
  const workspace = await getWorkspace(input.workspaceId);
  if (
    workspace === null ||
    workspace.status !== "running" ||
    workspace.activeTurnId !== input.turnId
  )
    return;

  const turn = workspace.messages.at(-1);
  const prompt =
    turn?.id === input.turnId && turn.role === "user" ? turn.text : undefined;
  let session: HarnessAgentSession | undefined;
  let text: string;
  let resumeState: HarnessAgentResumeSessionState;
  const dispatchId = `dispatch_${randomUUID().replaceAll("-", "")}`;
  const claimed = await mutateWorkspace(input.workspaceId, (current) =>
    current.activeTurnId === input.turnId &&
    current.activeDispatchId === undefined
      ? { ...current, activeDispatchId: dispatchId }
      : current
  );
  if (claimed.activeDispatchId !== dispatchId) {
    const now = new Date().toISOString();
    await mutateWorkspace(input.workspaceId, (current) =>
      current.activeTurnId === input.turnId
        ? {
            ...current,
            activeDispatchId: undefined,
            activeTurnId: undefined,
            error:
              "The turn was interrupted after dispatch. Inspect the transcript, diff, or sandbox before continuing.",
            sandbox: { ...current.sandbox, status: "running" },
            status: "error",
            updatedAt: now
          }
        : current
    );
    return;
  }
  try {
    if (!prompt || workspace.resumeState === undefined)
      throw new Error("Workspace turn is not initialized.");
    const agent = await createHarnessAgent(
      "opencode",
      "vercel",
      workspace.sessionId,
      {
        instructions: WORKSPACE_INSTRUCTIONS,
        resume: true
      }
    );
    session = await agent.createSession({
      sessionId: workspace.sessionId,
      resumeFrom: workspace.resumeState
    });
    const result = await agent.generate({ session, prompt });
    text = result.text;
    resumeState = await session.detach();
  } catch (error) {
    const resumeState = await session?.detach().catch(() => undefined);
    console.error("Workspace turn failed.", error);
    const now = new Date().toISOString();
    await mutateWorkspace(input.workspaceId, (current) => {
      if (current.activeTurnId !== input.turnId) return current;
      return {
        ...current,
        activeDispatchId: undefined,
        activeTurnId: undefined,
        error: "Workspace turn failed. Inspect the Workflow audit for details.",
        ...(resumeState === undefined ? {} : { resumeState }),
        sandbox: {
          ...current.sandbox,
          status: resumeState === undefined ? "error" : "running"
        },
        status: "error",
        updatedAt: now
      };
    });
    return;
  }

  const now = new Date().toISOString();
  await mutateWorkspace(input.workspaceId, (current) => {
    if (current.activeTurnId !== input.turnId) return current;
    const pullRequest = findPullRequest(text) ?? current.pullRequest;
    return {
      ...current,
      activeDispatchId: undefined,
      activeTurnId: undefined,
      error: undefined,
      messages: [
        ...current.messages,
        {
          createdAt: now,
          id: `msg_${input.turnId}`,
          role: "assistant" as const,
          text: text.slice(0, 100_000)
        }
      ].slice(-1000),
      resumeState,
      ...(pullRequest === undefined ? {} : { pullRequest }),
      sandbox: { ...current.sandbox, status: "running" },
      status: "idle",
      updatedAt: now
    };
  });
}

function findPullRequest(
  text: string
): { readonly number: number; readonly url: string } | undefined {
  const match = text.match(
    /https:\/\/github\.com\/vercel\/turborepo\/pull\/(\d+)/
  );
  if (!match) return;
  return { number: Number(match[1]), url: match[0] };
}

export async function workspaceTurnWorkflow(
  input: WorkspaceTurnInput
): Promise<void> {
  "use workflow";

  await ensureWorkspaceSession(input);
  await runWorkspaceTurn(input);
}
