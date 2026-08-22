import { BlobPreconditionFailedError, get, put } from "@vercel/blob";

import { strongBlobEtag } from "./blob-etag";
import { type AgentRunRecord, isAgentRunRecord } from "./run-types";

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

/**
 * Records the model a run is actually using. `session.started` carries no
 * model id, because a dynamic model resolves only once a step begins, so the
 * caller reports it per turn and this writes only when the model changed.
 */
export async function recordAgentRunModel(
  id: string,
  model: string
): Promise<void> {
  if (!isRunRegistryConfigured()) return;
  await mutateRuns((runs) => {
    const current = runs.find((run) => run.id === id);
    if (!current || current.model === model) return runs;
    return [
      { ...current, model, updatedAt: new Date().toISOString() },
      ...runs.filter((run) => run.id !== id)
    ];
  });
}

export async function getAgentRun(id: string): Promise<AgentRunRecord | null> {
  if (!isRunRegistryConfigured()) return null;
  return (await readRunIndex()).runs.find((run) => run.id === id) ?? null;
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
    etag: strongBlobEtag(result.blob.etag),
    runs: Array.isArray(value) ? value.filter(isAgentRunRecord) : []
  };
}

async function mutateRuns(
  mutation: (runs: AgentRunRecord[]) => AgentRunRecord[]
): Promise<void> {
  for (let attempt = 0; attempt < MAX_WRITE_ATTEMPTS; attempt += 1) {
    const current = await readRunIndex();
    const mutated = mutation(current.runs);
    // A mutation that returns its input changed nothing worth writing.
    if (mutated === current.runs) return;
    const runs = mutated
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
