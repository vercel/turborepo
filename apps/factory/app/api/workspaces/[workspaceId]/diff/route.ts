import {
  readWorkspaceDiff,
  WorkspaceDiffTooLargeError
} from "../../../../../agent/lib/workspace-diff";
import { getWorkspace } from "../../../../../agent/lib/workspace-store";

export async function GET(
  _request: Request,
  context: { params: Promise<{ workspaceId: string }> }
): Promise<Response> {
  const { workspaceId } = await context.params;
  const workspace = await getWorkspace(workspaceId);
  if (!workspace)
    return Response.json({ error: "Workspace not found." }, { status: 404 });
  if (!workspace.sandbox.id)
    return Response.json(
      { error: "The workspace sandbox is not ready yet." },
      { status: 409 }
    );

  try {
    return Response.json(
      { patch: await readWorkspaceDiff(workspace.sandbox.id) },
      { headers: { "cache-control": "no-store" } }
    );
  } catch (error) {
    if (error instanceof WorkspaceDiffTooLargeError) {
      return Response.json({ error: error.message }, { status: 413 });
    }
    console.error("Could not read workspace diff.", error);
    return Response.json(
      { error: "Could not read the workspace git diff." },
      { status: 502 }
    );
  }
}
