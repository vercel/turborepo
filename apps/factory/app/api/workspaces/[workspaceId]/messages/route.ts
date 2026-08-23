import {
  WorkspaceConflictError,
  WorkspaceNotFoundError
} from "../../../../../agent/lib/workspace-store";
import { queueWorkspaceTurn } from "../../../../../agent/lib/workspace-turn";
import {
  isWorkspaceMutationRequest,
  parseWorkspaceTurnInput,
  WORKSPACE_TURN_ACTION
} from "../../../../../agent/lib/workspace";

export async function POST(
  request: Request,
  context: { params: Promise<{ workspaceId: string }> }
): Promise<Response> {
  if (!isWorkspaceMutationRequest(request, WORKSPACE_TURN_ACTION))
    return Response.json(
      { error: "Invalid operator request." },
      { status: 403 }
    );
  const input = parseWorkspaceTurnInput(await request.json().catch(() => null));
  if (!input)
    return Response.json(
      { error: "A message between 1 and 20,000 characters is required." },
      { status: 400 }
    );

  const { workspaceId } = await context.params;
  try {
    return Response.json(await queueWorkspaceTurn(workspaceId, input.message), {
      status: 202,
      headers: { "cache-control": "no-store" }
    });
  } catch (error) {
    if (error instanceof WorkspaceNotFoundError)
      return Response.json({ error: error.message }, { status: 404 });
    if (error instanceof WorkspaceConflictError)
      return Response.json({ error: error.message }, { status: 409 });
    throw error;
  }
}
