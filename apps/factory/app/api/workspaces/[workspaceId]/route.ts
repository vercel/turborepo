import { getWorkspace } from "../../../../agent/lib/workspace-store";
import { toWorkspaceView } from "../../../../agent/lib/workspace";

export async function GET(
  _request: Request,
  context: { params: Promise<{ workspaceId: string }> }
): Promise<Response> {
  const { workspaceId } = await context.params;
  const workspace = await getWorkspace(workspaceId);
  if (!workspace)
    return Response.json({ error: "Workspace not found." }, { status: 404 });
  return Response.json(toWorkspaceView(workspace), {
    headers: { "cache-control": "no-store" }
  });
}
