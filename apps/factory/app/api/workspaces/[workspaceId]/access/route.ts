import { getOrCreateFxWorkspaceSandbox } from "../../../../../agent/lib/fx-workspace";
import { getWorkspace } from "../../../../../agent/lib/workspace-store";
import {
  isWorkspaceMutationRequest,
  WORKSPACE_ACCESS_ACTION
} from "../../../../../agent/lib/workspace";

export async function POST(
  request: Request,
  context: { params: Promise<{ workspaceId: string }> }
): Promise<Response> {
  if (
    !isWorkspaceMutationRequest(request, WORKSPACE_ACCESS_ACTION, {
      requireJson: false
    })
  )
    return Response.json(
      { error: "Invalid operator request." },
      { status: 403 }
    );
  const { workspaceId } = await context.params;
  const workspace = await getWorkspace(workspaceId);
  if (!workspace)
    return Response.json({ error: "Workspace not found." }, { status: 404 });
  await getOrCreateFxWorkspaceSandbox(workspace.sandbox.name);
  return Response.json(
    { sandboxName: workspace.sandbox.name },
    { headers: { "cache-control": "no-store" } }
  );
}
