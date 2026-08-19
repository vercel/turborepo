import { randomUUID } from "node:crypto";

import { start } from "workflow/api";

import { harnessExampleMaintenancePrompt } from "../../../../agent/lib/daily-example-maintenance";
import { listExamples } from "../../../../agent/lib/examples";
import { isHarnessId, isSandboxId } from "../../../../agent/lib/harnesses";
import { MAINTENANCE_RUN_ACTION } from "../../../../agent/lib/operator-runs";
import { harnessMaintenanceWorkflow } from "../../../../workflows/harness-maintenance";

export async function POST(request: Request): Promise<Response> {
  if (
    request.headers.get("origin") !== new URL(request.url).origin ||
    request.headers.get("content-type")?.split(";", 1)[0] !==
      "application/json" ||
    request.headers.get("x-operator-action") !== MAINTENANCE_RUN_ACTION
  )
    return Response.json(
      { error: "Invalid operator request." },
      { status: 403 }
    );

  if (!process.env.GITHUB_TOKEN_EXCHANGE_URL) {
    return Response.json(
      { error: "Harness workflow is not configured." },
      { status: 503 }
    );
  }

  const body: unknown = await request.json().catch(() => null);
  const input =
    typeof body === "object" && body !== null
      ? (body as Record<string, unknown>)
      : {};
  const example =
    typeof input.example === "string" && listExamples().includes(input.example)
      ? input.example
      : null;
  if (!example || !isHarnessId(input.harness) || !isSandboxId(input.sandbox)) {
    return Response.json(
      { error: "A valid example, harness, and sandbox are required." },
      { status: 400 }
    );
  }

  const sessionId = `ses_eve_${randomUUID().replaceAll("-", "")}`;
  const run = await start(harnessMaintenanceWorkflow, [
    {
      harness: input.harness,
      prompt: harnessExampleMaintenancePrompt(
        example,
        input.harness,
        sessionId
      ),
      sandbox: input.sandbox,
      sessionID: sessionId,
      title: `Maintain ${example}`
    }
  ]);
  return Response.json(
    {
      harness: input.harness,
      sandbox: input.sandbox,
      sessionId,
      state: "running",
      statusPath: `/api/harness/runs/${encodeURIComponent(sessionId)}?workflowRunId=${encodeURIComponent(run.runId)}`
    },
    {
      status: 202,
      headers: { "cache-control": "no-store" }
    }
  );
}
