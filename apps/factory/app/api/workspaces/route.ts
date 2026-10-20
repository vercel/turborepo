import {
  isWorkspaceStoreConfigured,
  listWorkspaces
} from "../../../agent/lib/workspace-store";
import { toWorkspaceSummary } from "../../../agent/lib/workspace";

export async function GET(): Promise<Response> {
  if (!isWorkspaceStoreConfigured()) return unconfigured();
  const workspaces = await listWorkspaces();
  return Response.json(
    { workspaces: workspaces.map(toWorkspaceSummary) },
    { headers: { "cache-control": "no-store" } }
  );
}

function unconfigured(): Response {
  return Response.json(
    { error: "Workspace storage is not configured." },
    { status: 503, headers: { "cache-control": "no-store" } }
  );
}
