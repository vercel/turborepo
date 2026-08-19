import type { HarnessId, SandboxId } from "../agent/lib/harnesses";

interface HarnessMaintenanceInput {
  readonly harness: HarnessId;
  readonly prompt: string;
  readonly sandbox: SandboxId;
  readonly sessionID: string;
  readonly title: string;
}

interface HarnessMaintenanceResult {
  readonly sessionID: string;
  readonly text: string;
}

async function runHarnessMaintenance(
  input: HarnessMaintenanceInput
): Promise<HarnessMaintenanceResult> {
  "use step";

  const { createHarnessAgent } = await import("../agent/lib/harness-agent");
  const { getAgentRun, updateAgentRun, writeAgentRun } =
    await import("../agent/lib/run-registry");
  async function record(task: Promise<void>) {
    try {
      await task;
    } catch (error) {
      console.error("Could not update the Harness run registry.", error);
    }
  }
  const existing = await getAgentRun(input.sessionID).catch(() => null);
  if (existing?.status === "completed") {
    return { sessionID: input.sessionID, text: "Run already completed." };
  }
  const now = new Date().toISOString();
  const sandboxId = `ai-sdk-harness-session-${input.sessionID}`;
  await record(
    writeAgentRun({
      agent: input.harness,
      harness: input.harness,
      id: input.sessionID,
      sandbox: {
        id: sandboxId,
        provider: input.sandbox,
        status: "provisioning"
      },
      source: "harness",
      startedAt: now,
      status: "running",
      title: input.title,
      trigger: "operator",
      updatedAt: now
    })
  );

  try {
    const agent = await createHarnessAgent(
      input.harness,
      input.sandbox,
      input.sessionID
    );
    const session = await agent.createSession({ sessionId: input.sessionID });
    await record(
      updateAgentRun(input.sessionID, {
        sandbox: { id: sandboxId, provider: input.sandbox, status: "running" }
      })
    );
    try {
      const result = await agent.generate({ session, prompt: input.prompt });
      await session.destroy();
      const finishedAt = new Date().toISOString();
      await record(
        updateAgentRun(input.sessionID, {
          finishedAt,
          sandbox: {
            id: sandboxId,
            provider: input.sandbox,
            status: "stopped"
          },
          status: "completed"
        })
      );
      return { sessionID: input.sessionID, text: result.text };
    } catch (error) {
      await session.destroy().catch(() => {});
      throw error;
    }
  } catch (error) {
    await record(
      updateAgentRun(input.sessionID, {
        finishedAt: new Date().toISOString(),
        sandbox: {
          id: sandboxId,
          provider: input.sandbox,
          status: "failed"
        },
        status: "failed"
      })
    );
    throw error;
  }
}

export async function harnessMaintenanceWorkflow(
  input: HarnessMaintenanceInput
): Promise<HarnessMaintenanceResult> {
  "use workflow";

  return runHarnessMaintenance(input);
}
