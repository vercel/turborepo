import { randomUUID } from "node:crypto";

import { fxExampleMaintenancePrompt } from "../../../../agent/lib/daily-example-maintenance";
import { listExamples } from "../../../../agent/lib/examples";
import { MAINTENANCE_RUN_ACTION } from "../../../../agent/lib/operator-runs";
import { createFxWorkspace } from "../../../../agent/lib/workspace-turn";
import { isWorkspaceMutationRequest } from "../../../../agent/lib/workspace";

export async function POST(request: Request): Promise<Response> {
  if (!isWorkspaceMutationRequest(request, MAINTENANCE_RUN_ACTION))
    return Response.json(
      { error: "Invalid operator request." },
      { status: 403 }
    );
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

  const runId = randomUUID().replaceAll("-", "");
  const workspace = await createFxWorkspace({
    prompt: fxExampleMaintenancePrompt(example, runId),
    title: `Maintain ${example}`
  });
  return Response.json(workspace, {
    status: 202,
    headers: { "cache-control": "no-store" }
  });
}
