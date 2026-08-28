import {
  isWorkspaceStoreConfigured,
  listWorkspaces
} from "../../../agent/lib/workspace-store";
import { reconcileWorkspacePullRequests } from "../../../agent/lib/workspace-pull-request";
import { createFxWorkspace } from "../../../agent/lib/workspace-turn";
import {
  isWorkspaceMutationRequest,
  parseCreateWorkspaceInput,
  toWorkspaceSummary,
  WORKSPACE_CREATE_ACTION
} from "../../../agent/lib/workspace";

export async function GET(): Promise<Response> {
  if (!isWorkspaceStoreConfigured()) return unconfigured();
  const workspaces = await reconcileWorkspacePullRequests(
    await listWorkspaces()
  );
  return Response.json(
    { workspaces: workspaces.map(toWorkspaceSummary) },
    { headers: { "cache-control": "no-store" } }
  );
}

export async function POST(request: Request): Promise<Response> {
  if (!isWorkspaceMutationRequest(request, WORKSPACE_CREATE_ACTION))
    return Response.json(
      { error: "Invalid operator request." },
      { status: 403 }
    );
  if (!isWorkspaceStoreConfigured()) return unconfigured();

  const input = parseCreateWorkspaceInput(
    await request.json().catch(() => null)
  );
  if (!input)
    return Response.json(
      { error: "A title or prompt is required." },
      { status: 400 }
    );

  const view = await createFxWorkspace(input);
  return Response.json(view, {
    status: input.prompt ? 202 : 201,
    headers: { "cache-control": "no-store" }
  });
}

function unconfigured(): Response {
  return Response.json(
    { error: "Workspace storage is not configured." },
    { status: 503, headers: { "cache-control": "no-store" } }
  );
}
