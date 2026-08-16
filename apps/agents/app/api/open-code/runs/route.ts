import { randomUUID } from "node:crypto";

import { start } from "workflow/api";

import { DAILY_EXAMPLE_MAINTENANCE_PROMPT } from "../../../../agent/lib/daily-example-maintenance";
import { listExamples } from "../../../../agent/lib/examples";
import {
  MAINTENANCE_RUN_ACTION,
  signWorkflowRun
} from "../../../../agent/lib/operator-runs";
import { openCodeMaintenanceWorkflow } from "../../../../workflows/open-code-maintenance";

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

  const runSecret = process.env.OPERATOR_RUN_SECRET;
  if (!process.env.OPENCODE_SERVER_URL || !runSecret) {
    return Response.json(
      { error: "OpenCode workflow is not configured." },
      { status: 503 }
    );
  }

  const body: unknown = await request.json().catch(() => null);
  const example =
    typeof body === "object" &&
    body !== null &&
    "example" in body &&
    typeof body.example === "string" &&
    listExamples().includes(body.example)
      ? body.example
      : null;
  if (!example)
    return Response.json(
      { error: "A valid example is required." },
      { status: 400 }
    );

  const sessionId = `ses_eve_${randomUUID().replaceAll("-", "")}`;
  const run = await start(openCodeMaintenanceWorkflow, [
    {
      prompt: `${DAILY_EXAMPLE_MAINTENANCE_PROMPT}\n\nMaintain only the ${example} example.`,
      sessionID: sessionId,
      title: `[Eve · Operator] Maintain ${example}`
    }
  ]);
  const token = signWorkflowRun(
    { sessionID: sessionId, workflowRunID: run.runId },
    runSecret
  );
  return Response.json(
    {
      sessionId,
      state: "running",
      statusPath: `/api/open-code/runs/${encodeURIComponent(sessionId)}?token=${encodeURIComponent(token)}`
    },
    { status: 202, headers: { "cache-control": "no-store" } }
  );
}
