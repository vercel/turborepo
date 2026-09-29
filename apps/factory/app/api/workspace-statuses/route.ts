import { workspaceDisplayStatuses } from "../../../agent/lib/workspace-pull-request";
import { isWorkspaceStoreConfigured } from "../../../agent/lib/workspace-store";

export async function GET(): Promise<Response> {
  if (!isWorkspaceStoreConfigured())
    return Response.json(
      { error: "Workspace storage is not configured." },
      { status: 503, headers: { "cache-control": "no-store" } }
    );
  return Response.json(
    { statuses: await workspaceDisplayStatuses() },
    { headers: { "cache-control": "no-store" } }
  );
}
