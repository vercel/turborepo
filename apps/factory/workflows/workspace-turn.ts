import { randomUUID } from "node:crypto";

interface WorkspaceTurnInput {
  readonly turnId: string;
  readonly workspaceId: string;
}

const FIRST_TURN_INSTRUCTIONS = `You are working interactively with a Turborepo maintainer in the vercel/turborepo checkout. Preserve all work in the checkout, make the smallest correct change, verify it using the repository's own commands, and report concrete results. Do not open a pull request unless the maintainer asks. When asked, create an agents/<topic> branch, use a Conventional Commit title with an uppercase description and no scope, push it, and open a draft pull request with validation results in the body.`;

async function runWorkspaceTurn(input: WorkspaceTurnInput): Promise<void> {
  "use step";

  const { getOrCreateFxWorkspaceSandbox, runFxTurn } =
    await import("../agent/lib/fx-workspace");
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
  const message =
    turn?.id === input.turnId && turn.role === "user" ? turn.text : undefined;
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
              "The fx turn was interrupted after dispatch. Inspect the transcript, diff, or sandbox before continuing.",
            sandbox: { ...current.sandbox, status: "running" },
            status: "error",
            updatedAt: now
          }
        : current
    );
    return;
  }

  try {
    if (!message) throw new Error("Workspace turn prompt is missing.");
    const sandbox = await getOrCreateFxWorkspaceSandbox(workspace.sandbox.name);
    const prompt = workspace.sessionId
      ? message
      : `${FIRST_TURN_INSTRUCTIONS}\n\nMaintainer request:\n${message}`;
    const result = await runFxTurn(
      sandbox,
      prompt,
      workspace.sessionId,
      undefined,
      async (sessionId) => {
        await mutateWorkspace(input.workspaceId, (current) =>
          current.activeTurnId === input.turnId
            ? { ...current, sessionId }
            : current
        );
      }
    );
    const now = new Date().toISOString();
    await mutateWorkspace(input.workspaceId, (current) => {
      if (current.activeTurnId !== input.turnId) return current;
      const pullRequest = findPullRequest(result.output) ?? current.pullRequest;
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
            text: result.output.slice(0, 100_000)
          }
        ].slice(-1000),
        ...(pullRequest === undefined ? {} : { pullRequest }),
        sandbox: { ...current.sandbox, status: "running" },
        sessionId: result.sessionId,
        status: "idle",
        updatedAt: now
      };
    });
  } catch (error) {
    console.error("fx workspace turn failed.", error);
    const now = new Date().toISOString();
    await mutateWorkspace(input.workspaceId, (current) =>
      current.activeTurnId === input.turnId
        ? {
            ...current,
            activeDispatchId: undefined,
            activeTurnId: undefined,
            error:
              "fx workspace turn failed. Inspect the Workflow audit for details.",
            sandbox: { ...current.sandbox, status: "error" },
            status: "error",
            updatedAt: now
          }
        : current
    );
  }
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

  await runWorkspaceTurn(input);
}
