/**
 * Durable ledger for factory image builds, stored in the same private
 * Vercel Blob store as the agent run registry.
 *
 * Writes are compare-and-swap on the blob etag, so two merges landing at
 * the same moment cannot both claim the ledger: the loser retries against
 * the winner's state and is either deduplicated or supersedes it.
 */

import { BlobPreconditionFailedError, get, put } from "@vercel/blob";

import { factoryImageFingerprint } from "./factory-image";
import {
  type FactoryImageBuild,
  type FactoryImageBuildChanges,
  type FactoryImageClaim,
  type FactoryImageClaimInput,
  type FactoryImagePointer,
  type FactoryImagePublishInput,
  type FactoryImageState,
  type FactoryImageView,
  claimFactoryImageBuild,
  EMPTY_FACTORY_IMAGE_STATE,
  findFactoryImageBuild,
  parseFactoryImageState,
  publishFactoryImagePointer,
  updateFactoryImageBuild
} from "./factory-image-types";

const STATE_PATH = "factory-image/v1/state.json";
const MAX_WRITE_ATTEMPTS = 5;
/** Builds surfaced on the operator dashboard. */
const VIEW_BUILDS = 8;

export function isFactoryImageRegistryConfigured(): boolean {
  return Boolean(
    (process.env.BLOB_STORE_ID && process.env.VERCEL_OIDC_TOKEN) ||
    process.env.BLOB_READ_WRITE_TOKEN
  );
}

export async function readFactoryImageState(): Promise<FactoryImageState> {
  if (!isFactoryImageRegistryConfigured()) return EMPTY_FACTORY_IMAGE_STATE;
  return (await readState()).state;
}

/**
 * Snapshot every consumer boots from, or `null` when no image has been
 * published yet. Callers must keep working without one.
 */
export async function readFactoryImagePointer(): Promise<FactoryImagePointer | null> {
  try {
    return (await readFactoryImageState()).pointer;
  } catch (error) {
    console.error("Could not read the factory image pointer.", error);
    return null;
  }
}

export async function readFactoryImageBuild(
  buildId: string
): Promise<FactoryImageBuild | null> {
  return findFactoryImageBuild(await readFactoryImageState(), buildId);
}

/** Ledger projection for the operator dashboard. Never throws. */
export async function readFactoryImageView(): Promise<FactoryImageView> {
  const configured = isFactoryImageRegistryConfigured();
  const state = configured
    ? await readFactoryImageState().catch((error: unknown) => {
        console.error("Could not read the factory image ledger.", error);
        return EMPTY_FACTORY_IMAGE_STATE;
      })
    : EMPTY_FACTORY_IMAGE_STATE;
  return {
    builds: state.builds.slice(0, VIEW_BUILDS),
    configured,
    fingerprint: factoryImageFingerprint(),
    pointer: state.pointer
  };
}

export async function claimFactoryImage(
  input: FactoryImageClaimInput
): Promise<FactoryImageClaim> {
  return mutate((state) => {
    const claim = claimFactoryImageBuild(state, input);
    return {
      next: claim.kind === "claimed" ? claim.state : state,
      result: claim
    };
  });
}

export async function recordFactoryImageProgress(
  buildId: string,
  changes: FactoryImageBuildChanges
): Promise<FactoryImageBuild | null> {
  return mutate((state) => {
    const next = updateFactoryImageBuild(
      state,
      buildId,
      changes,
      new Date().toISOString()
    );
    return { next, result: findFactoryImageBuild(next, buildId) };
  });
}

export async function publishFactoryImage(
  buildId: string,
  input: Omit<FactoryImagePublishInput, "now">
): Promise<FactoryImagePointer | null> {
  return mutate((state) => {
    const outcome = publishFactoryImagePointer(state, buildId, {
      ...input,
      now: new Date().toISOString()
    });
    return {
      next: outcome.state,
      result: outcome.published ? outcome.state.pointer : null
    };
  });
}

async function readState(): Promise<{
  etag?: string;
  state: FactoryImageState;
}> {
  const result = await get(STATE_PATH, { access: "private", useCache: false });
  if (!result || result.statusCode !== 200) {
    return { state: EMPTY_FACTORY_IMAGE_STATE };
  }
  const value: unknown = await new Response(result.stream).json();
  return { etag: result.blob.etag, state: parseFactoryImageState(value) };
}

async function mutate<T>(
  mutation: (state: FactoryImageState) => {
    next: FactoryImageState;
    result: T;
  }
): Promise<T> {
  if (!isFactoryImageRegistryConfigured()) {
    throw new Error(
      "The factory image registry requires a private Vercel Blob store."
    );
  }
  for (let attempt = 0; attempt < MAX_WRITE_ATTEMPTS; attempt += 1) {
    const current = await readState();
    const { next, result } = mutation(current.state);
    if (next === current.state) return result;
    try {
      await put(STATE_PATH, JSON.stringify(next), {
        access: "private",
        allowOverwrite: current.etag !== undefined,
        contentType: "application/json",
        ifMatch: current.etag
      });
      return result;
    } catch (error) {
      if (error instanceof BlobPreconditionFailedError) continue;
      if (current.etag === undefined && (await readState()).etag) continue;
      throw error;
    }
  }
  throw new Error(
    "The factory image ledger changed too frequently; retry the update."
  );
}
