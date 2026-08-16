import { getRun } from "workflow/api";

import { verifyWorkflowRun } from "../../../../../agent/lib/operator-runs";

export async function GET(
  request: Request,
  context: { params: Promise<{ sessionId: string }> }
): Promise<Response> {
  const { sessionId } = await context.params;
  const secret = process.env.OPERATOR_RUN_SECRET;
  const token = new URL(request.url).searchParams.get("token");
  const workflowRunId =
    secret && token ? verifyWorkflowRun(token, sessionId, secret) : null;
  if (!workflowRunId)
    return Response.json({ error: "Invalid workflow run." }, { status: 403 });

  const status = await getRun(workflowRunId).status;
  const state =
    status === "completed"
      ? "done"
      : status === "failed" || status === "cancelled"
        ? "error"
        : "running";
  return Response.json(
    { sessionId, state },
    { headers: { "cache-control": "no-store" } }
  );
}
