import {
  HarnessAgent,
  type HarnessAgentAdapter,
  type HarnessAgentResumeSessionState
} from "@ai-sdk/harness/agent";
import { createClaudeCode } from "@ai-sdk/harness-claude-code";
import { createCodex } from "@ai-sdk/harness-codex";
import { createCursor } from "@ai-sdk/harness-cursor";
import { createFx } from "@ai-sdk/harness-fx";
import { createOpenCode } from "@ai-sdk/harness-opencode";
import { createPi } from "@ai-sdk/harness-pi";
import { createVercelSandbox } from "@ai-sdk/sandbox-vercel";
import { Sandbox } from "@vercel/sandbox";
import type { RuntimeSandboxSession } from "eve/sandbox";

import {
  FACTORY_HARNESS_PORT,
  FACTORY_HARNESS_WORKDIR
} from "./harness-agent-config";
import { parseHarnessResumeState } from "./harness-agent-state";
import { DEFAULT_WORKSPACE_HARNESS, type WorkspaceHarness } from "./workspace";

const CODING_AGENT_INSTRUCTIONS = `You are the coding agent for the Turborepo Factory. Work directly in the current Turborepo checkout. Follow repository instructions, make the smallest correct changes, and run relevant validation. Never commit, push, or create a pull request; the parent Factory agent owns those operations and credentials.`;

export interface RunFactoryHarnessAgentInput {
  readonly abortSignal?: AbortSignal;
  readonly harness?: WorkspaceHarness;
  readonly model?: string;
  readonly prompt: string;
  readonly sandbox: RuntimeSandboxSession;
  readonly sessionId: string;
}

/**
 * Runs the selected coding harness through AI SDK HarnessAgent in the current
 * Eve sandbox. Eve retains ownership of the sandbox; HarnessAgent owns only the
 * coding runtime it starts.
 */
export async function runFactoryHarnessAgent({
  abortSignal,
  harness = DEFAULT_WORKSPACE_HARNESS,
  model,
  prompt,
  sandbox,
  sessionId
}: RunFactoryHarnessAgentInput): Promise<{
  readonly harness: WorkspaceHarness;
  readonly sessionId: string;
  readonly text: string;
}> {
  const nativeSandbox = await Sandbox.get({ name: sandbox.id, resume: true });
  // The adapter currently resolves its own compatible @vercel/sandbox copy.
  // Both versions expose the same runtime Sandbox surface used by the wrapper.
  const sandboxProvider = createVercelSandbox({
    sandbox: nativeSandbox as never
  });
  const sandboxSession = await sandboxProvider.createSession({
    abortSignal,
    sessionId
  });
  const agent = new HarnessAgent({
    harness: createFactoryHarness(harness, model),
    instructions: CODING_AGENT_INSTRUCTIONS,
    permissionMode: "allow-all",
    sandboxConfig: { workDir: FACTORY_HARNESS_WORKDIR }
  });
  const resumeFrom = await readResumeState(sandbox, harness);
  const session = await agent.createSession({
    abortSignal,
    ...(resumeFrom === undefined ? {} : { resumeFrom }),
    sandboxSession,
    sessionId
  });

  try {
    const result = session.hasUnfinishedTurn()
      ? await agent.continueGenerate({ abortSignal, session })
      : await agent.generate({ abortSignal, prompt, session });
    await persistResumeState(sandbox, harness, await session.stop());
    return { harness, sessionId: session.sessionId, text: result.text };
  } catch (error) {
    await session.destroy().catch(() => undefined);
    throw error;
  }
}

export function createFactoryHarness(
  harness: WorkspaceHarness,
  model?: string
): HarnessAgentAdapter {
  switch (harness) {
    case "claude-code": {
      const selectedModel = providerModel(model, "anthropic");
      return createClaudeCode({
        auth: "ai-gateway",
        ...(selectedModel === undefined ? {} : { model: selectedModel }),
        port: FACTORY_HARNESS_PORT
      });
    }
    case "codex": {
      const selectedModel = providerModel(model, "openai");
      return createCodex({
        auth: "ai-gateway",
        ...(selectedModel === undefined ? {} : { model: selectedModel }),
        port: FACTORY_HARNESS_PORT,
        webSearch: true
      });
    }
    case "cursor":
      // Cursor controls provider routing itself. Keep its native default model.
      return createCursor({ auth: "auto", port: FACTORY_HARNESS_PORT });
    case "opencode": {
      const selected = splitGatewayModel(model);
      return createOpenCode({
        auth: "ai-gateway",
        ...(selected === undefined
          ? {}
          : { model: selected.model, provider: selected.provider }),
        port: FACTORY_HARNESS_PORT
      });
    }
    case "pi":
      return createPi({
        auth: "ai-gateway",
        ...(model === undefined ? {} : { model })
      });
    case "fx":
      return createFx({
        auth: "ai-gateway",
        ...(model === undefined ? {} : { model }),
        port: FACTORY_HARNESS_PORT
      });
  }
}

export function providerModel(
  model: string | undefined,
  provider: string
): string | undefined {
  const selected = splitGatewayModel(model);
  return selected?.provider === provider ? selected.model : undefined;
}

function splitGatewayModel(
  model: string | undefined
): { model: string; provider: string } | undefined {
  if (model === undefined) return undefined;
  const slash = model.indexOf("/");
  return slash > 0
    ? { model: model.slice(slash + 1), provider: model.slice(0, slash) }
    : undefined;
}

function resumeStatePath(harness: WorkspaceHarness): string {
  return `/factory/state/harness-agent-${harness}.json`;
}

async function readResumeState(
  sandbox: RuntimeSandboxSession,
  harness: WorkspaceHarness
): Promise<HarnessAgentResumeSessionState | undefined> {
  try {
    const value = await sandbox.readTextFile({
      path: resumeStatePath(harness)
    });
    return value === null
      ? undefined
      : (parseHarnessResumeState(value) as
          | HarnessAgentResumeSessionState
          | undefined);
  } catch {
    return undefined;
  }
}

async function persistResumeState(
  sandbox: RuntimeSandboxSession,
  harness: WorkspaceHarness,
  state: HarnessAgentResumeSessionState
): Promise<void> {
  const result = await sandbox.run({ command: "mkdir -p /factory/state" });
  if (result.exitCode !== 0) {
    throw new Error("Could not prepare HarnessAgent session storage.");
  }
  await sandbox.writeTextFile({
    content: JSON.stringify(state),
    path: resumeStatePath(harness)
  });
}
