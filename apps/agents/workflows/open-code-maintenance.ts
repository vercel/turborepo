interface OpenCodeMaintenanceInput {
  readonly prompt: string;
  readonly sessionID: string;
  readonly title: string;
}

interface OpenCodeMaintenanceResult {
  readonly sessionID: string;
  readonly text: string;
}

async function runOpenCodeMaintenance(
  input: OpenCodeMaintenanceInput
): Promise<OpenCodeMaintenanceResult> {
  "use step";

  const { createRemoteOpenCodeAgent } = await import(
    "../agent/lib/remote-opencode-agent"
  );
  const agent = createRemoteOpenCodeAgent(input.title);
  const session = await agent.createSession({ sessionId: input.sessionID });
  try {
    const result = await agent.generate({ session, prompt: input.prompt });
    return { sessionID: input.sessionID, text: result.text };
  } finally {
    await session.stop();
  }
}

export async function openCodeMaintenanceWorkflow(
  input: OpenCodeMaintenanceInput
): Promise<OpenCodeMaintenanceResult> {
  "use workflow";

  return runOpenCodeMaintenance(input);
}
