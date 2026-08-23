import { Sandbox } from "@vercel/sandbox";

import { FACTORY_IMAGE_SPEC } from "../../../../../agent/lib/factory-image";
import { getWorkspace } from "../../../../../agent/lib/workspace-store";

const MAX_AUDIT_LENGTH = 2_000_000;

export async function GET(
  _request: Request,
  context: { params: Promise<{ workspaceId: string }> }
): Promise<Response> {
  const { workspaceId } = await context.params;
  const workspace = await getWorkspace(workspaceId);
  if (!workspace)
    return Response.json({ error: "Workspace not found." }, { status: 404 });
  if (!workspace.sessionId)
    return Response.json(
      { error: "The fx session has not started yet." },
      { status: 409 }
    );

  const sandbox = await Sandbox.get({ name: workspace.sandbox.name });
  const command = await sandbox.runCommand({
    args: ["session", "--id", workspace.sessionId, "--json"],
    cmd: "fx",
    cwd: FACTORY_IMAGE_SPEC.checkoutPath,
    env: { FX_AUTO_UPGRADE: "0" },
    timeoutMs: 30_000
  });
  if (command.exitCode !== 0) {
    return Response.json(
      { error: "Could not read the fx session audit." },
      { status: 502 }
    );
  }
  const audit = await command.stdout();
  return Response.json(
    {
      audit: audit.slice(0, MAX_AUDIT_LENGTH),
      truncated: audit.length > MAX_AUDIT_LENGTH
    },
    { headers: { "cache-control": "no-store" } }
  );
}
