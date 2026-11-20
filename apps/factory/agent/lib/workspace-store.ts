import { BlobPreconditionFailedError, get, list, put } from "@vercel/blob";

import { strongBlobEtag } from "./blob-etag";
import {
  isWorkspaceId,
  isWorkspaceRecord,
  type WorkspaceRecord
} from "./workspace";

const PREFIX = "factory-workspaces/v1/";
const MAX_WRITE_ATTEMPTS = 5;

export class WorkspaceConflictError extends Error {}

export function isWorkspaceStoreConfigured(): boolean {
  return Boolean(
    (process.env.BLOB_STORE_ID && process.env.VERCEL_OIDC_TOKEN) ||
    process.env.BLOB_READ_WRITE_TOKEN
  );
}

export async function createWorkspace(
  workspace: WorkspaceRecord
): Promise<WorkspaceRecord> {
  if (!isWorkspaceRecord(workspace))
    throw new Error("Invalid workspace record.");
  try {
    await put(pathFor(workspace.id), JSON.stringify(workspace), {
      access: "private",
      addRandomSuffix: false,
      contentType: "application/json"
    });
    return workspace;
  } catch (error) {
    if (error instanceof BlobPreconditionFailedError)
      throw new WorkspaceConflictError("Workspace already exists.");
    throw error;
  }
}

export async function getWorkspace(
  id: string
): Promise<WorkspaceRecord | null> {
  return (await readWorkspace(id)).workspace;
}

export async function listWorkspaces(): Promise<WorkspaceRecord[]> {
  const workspaces: WorkspaceRecord[] = [];
  let cursor: string | undefined;
  do {
    const page = await list({ cursor, limit: 100, prefix: PREFIX });
    const records = await Promise.all(
      page.blobs.map((blob) => readWorkspace(pathId(blob.pathname)))
    );
    workspaces.push(
      ...records.flatMap(({ workspace }) => (workspace ? [workspace] : []))
    );
    cursor = page.hasMore ? page.cursor : undefined;
  } while (cursor);
  return workspaces.sort((left, right) =>
    right.updatedAt.localeCompare(left.updatedAt)
  );
}

export async function mutateWorkspace(
  id: string,
  mutation: (workspace: WorkspaceRecord) => WorkspaceRecord | null
): Promise<WorkspaceRecord> {
  for (let attempt = 0; attempt < MAX_WRITE_ATTEMPTS; attempt += 1) {
    const current = await readWorkspace(id);
    if (!current.workspace || !current.etag)
      throw new Error("Workspace not found.");
    const workspace = mutation(current.workspace);
    if (workspace === null)
      throw new WorkspaceConflictError("Workspace is already running a turn.");
    if (!isWorkspaceRecord(workspace) || workspace.id !== id)
      throw new Error("Invalid workspace mutation.");
    try {
      await put(pathFor(id), JSON.stringify(workspace), {
        access: "private",
        allowOverwrite: true,
        contentType: "application/json",
        ifMatch: current.etag
      });
      return workspace;
    } catch (error) {
      if (error instanceof BlobPreconditionFailedError) continue;
      throw error;
    }
  }
  throw new WorkspaceConflictError(
    "Workspace changed too frequently; retry the request."
  );
}

async function readWorkspace(id: string): Promise<{
  readonly etag?: string;
  readonly workspace: WorkspaceRecord | null;
}> {
  if (!isWorkspaceId(id)) return { workspace: null };
  const result = await get(pathFor(id), { access: "private", useCache: false });
  if (!result || result.statusCode !== 200) return { workspace: null };
  const value: unknown = await new Response(result.stream)
    .json()
    .catch(() => null);
  return {
    etag: strongBlobEtag(result.blob.etag),
    workspace: isWorkspaceRecord(value) ? value : null
  };
}

function pathFor(id: string): string {
  return `${PREFIX}${id}.json`;
}

function pathId(pathname: string): string {
  return pathname.slice(PREFIX.length, -".json".length);
}
