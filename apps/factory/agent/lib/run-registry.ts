import { BlobPreconditionFailedError, get, put } from "@vercel/blob";
import { Sandbox } from "@vercel/sandbox";

import {
  type AgentRunRecord,
  type ControlPlaneSnapshot,
  isAgentRunRecord,
  type SandboxResource
} from "./run-types";

const RUN_INDEX_PATH = "agent-runs/v1/index.json";
const MAX_RUNS = 100;
const MAX_WRITE_ATTEMPTS = 3;

export function isRunRegistryConfigured(): boolean {
  return Boolean(
    (process.env.BLOB_STORE_ID && process.env.VERCEL_OIDC_TOKEN) ||
    process.env.BLOB_READ_WRITE_TOKEN
  );
}

export async function writeAgentRun(run: AgentRunRecord): Promise<void> {
  if (!isRunRegistryConfigured()) return;
  await mutateRuns((runs) => [
    run,
    ...runs.filter((candidate) => candidate.id !== run.id)
  ]);
}

export async function updateAgentRun(
  id: string,
  changes: Partial<AgentRunRecord>
): Promise<void> {
  if (!isRunRegistryConfigured()) return;
  await mutateRuns((runs) => {
    const current = runs.find((run) => run.id === id);
    if (!current) return runs;
    return [
      {
        ...current,
        ...changes,
        id,
        updatedAt: new Date().toISOString()
      },
      ...runs.filter((run) => run.id !== id)
    ];
  });
}

export async function getAgentRun(id: string): Promise<AgentRunRecord | null> {
  if (!isRunRegistryConfigured()) return null;
  return (await readRunIndex()).runs.find((run) => run.id === id) ?? null;
}

export async function listControlPlaneSnapshot(): Promise<ControlPlaneSnapshot> {
  const configured = isRunRegistryConfigured();
  if (!configured) {
    const inventory = await listSandboxResources();
    return {
      configured,
      runs: [],
      sandboxError: inventory.error,
      sandboxes: inventory.resources
    };
  }
  let runError = false;
  const [runs, inventory] = await Promise.all([
    listAgentRuns().catch((error: unknown) => {
      console.error("Could not load the agent run registry.", error);
      runError = true;
      return [];
    }),
    listSandboxResources()
  ]);
  return {
    configured,
    error: runError ? "Agent run history is currently unavailable." : undefined,
    runs,
    sandboxError: inventory.error,
    sandboxes: inventory.resources
  };
}

async function readRunIndex(): Promise<{
  etag?: string;
  runs: AgentRunRecord[];
}> {
  const result = await get(RUN_INDEX_PATH, {
    access: "private",
    useCache: false
  });
  if (!result || result.statusCode !== 200) return { runs: [] };
  const value: unknown = await new Response(result.stream).json();
  return {
    etag: result.blob.etag,
    runs: Array.isArray(value) ? value.filter(isAgentRunRecord) : []
  };
}

async function listAgentRuns(): Promise<AgentRunRecord[]> {
  return (await readRunIndex()).runs.sort((left, right) =>
    right.startedAt.localeCompare(left.startedAt)
  );
}

async function mutateRuns(
  mutation: (runs: AgentRunRecord[]) => AgentRunRecord[]
): Promise<void> {
  for (let attempt = 0; attempt < MAX_WRITE_ATTEMPTS; attempt += 1) {
    const current = await readRunIndex();
    const runs = mutation(current.runs)
      .sort((left, right) => right.startedAt.localeCompare(left.startedAt))
      .slice(0, MAX_RUNS);
    try {
      await put(RUN_INDEX_PATH, JSON.stringify(runs), {
        access: "private",
        allowOverwrite: current.etag !== undefined,
        contentType: "application/json",
        ifMatch: current.etag
      });
      return;
    } catch (error) {
      if (error instanceof BlobPreconditionFailedError) continue;
      if (current.etag === undefined && (await readRunIndex()).etag) continue;
      throw error;
    }
  }
  throw new Error(
    "Agent run registry changed too frequently; retry the update."
  );
}

async function listSandboxResources(): Promise<{
  error?: boolean;
  resources: SandboxResource[];
}> {
  try {
    const result = await Sandbox.list({
      limit: 50,
      namePrefix: "ai-sdk-harness",
      sortBy: "name",
      sortOrder: "asc"
    });
    const sandboxes = await result.toArray();
    return {
      resources: sandboxes
        .map((sandbox) => ({
          createdAt: sandbox.createdAt,
          name: sandbox.name,
          region: sandbox.region,
          runtime: sandbox.runtime,
          status: sandbox.status,
          updatedAt: sandbox.updatedAt
        }))
        .sort((left, right) => right.updatedAt - left.updatedAt)
    };
  } catch {
    return { error: true, resources: [] };
  }
}
