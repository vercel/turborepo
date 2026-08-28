import { getFxWorkspaceSandbox } from "../../../../../agent/lib/fx-workspace";
import {
  getWorkspace,
  mutateWorkspace
} from "../../../../../agent/lib/workspace-store";
import {
  isWorkspacePublishRequest,
  parseWorkspacePublishInput,
  publishWorkspacePullRequest,
  workspacePublishBridge
} from "../../../../../agent/lib/workspace-publish";

export async function POST(
  request: Request,
  context: { params: Promise<{ workspaceId: string }> }
): Promise<Response> {
  const { workspaceId } = await context.params;
  const workspace = await getWorkspace(workspaceId);
  if (!workspace || !isWorkspacePublishRequest(request, workspace))
    return Response.json(
      { error: "Workspace publication is not authorized." },
      { status: 403 }
    );

  const input = parseWorkspacePublishInput(
    await request.json().catch(() => null)
  );
  if (!input)
    return Response.json(
      { error: "Invalid pull request metadata." },
      { status: 400 }
    );

  try {
    const bridge = workspace.publishToken
      ? workspacePublishBridge(workspace.id, workspace.publishToken)
      : null;
    const sandbox = await getFxWorkspaceSandbox(workspace.sandbox.name, bridge);
    const result = await publishWorkspacePullRequest(sandbox, workspace, input);
    if (typeof result.number === "number" && typeof result.url === "string") {
      const now = new Date().toISOString();
      await mutateWorkspace(workspaceId, (current) => ({
        ...current,
        activity: current.status === "running" ? current.activity : undefined,
        completeAfterTurn:
          current.pullRequest?.number === result.number &&
          current.pullRequest.state === "merged"
            ? current.completeAfterTurn
            : undefined,
        pullRequest:
          current.pullRequest?.number === result.number &&
          current.pullRequest.state === "merged"
            ? current.pullRequest
            : {
                checkedAt: now,
                number: result.number,
                state: "open",
                url: result.url
              },
        status:
          current.pullRequest?.number === result.number &&
          current.pullRequest.state === "merged"
            ? current.status === "running"
              ? "running"
              : "done"
            : current.status === "done"
              ? "idle"
              : current.status,
        updatedAt: now
      }));
    }
    return Response.json(result, { headers: { "cache-control": "no-store" } });
  } catch (error) {
    console.error("Could not publish workspace pull request.", error);
    return Response.json(
      {
        error:
          error instanceof Error
            ? error.message
            : "Could not publish pull request."
      },
      { status: 422, headers: { "cache-control": "no-store" } }
    );
  }
}
