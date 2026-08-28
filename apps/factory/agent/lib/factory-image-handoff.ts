/**
 * Build-time handoff of the published factory image to the Eve sandbox
 * backend.
 *
 * Eve resolves `revalidationKey` asynchronously while it compiles, then asks
 * the sandbox definition for its backend synchronously when it prewarms the
 * template. Reading the image pointer needs `await`, so the async hook records
 * the latest published snapshot here and the synchronous factory picks it up.
 */

import { readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";

export interface FactoryImageHandoff {
  readonly commit: string;
  readonly fingerprint: string;
  readonly snapshotId: string;
}

interface HandoffFile {
  readonly commit?: string;
  readonly fingerprint?: string;
  readonly snapshotId?: string;
}

const HANDOFF_PATH =
  process.env.FACTORY_IMAGE_HANDOFF_PATH ??
  path.join(tmpdir(), "turborepo-factory-image-base.json");

let cached: HandoffFile | null | undefined;

export function writeFactoryImageHandoff(handoff: {
  readonly commit?: string;
  readonly fingerprint?: string;
  readonly snapshotId?: string;
}): void {
  if (
    handoff.commit === undefined ||
    handoff.fingerprint === undefined ||
    handoff.snapshotId === undefined
  )
    return;
  cached = handoff;
  try {
    writeFileSync(HANDOFF_PATH, JSON.stringify(handoff), "utf8");
  } catch (error) {
    console.warn("Could not record the factory image snapshot.", error);
  }
}

/** Latest published snapshot recorded during this deployment build. */
export function readFactoryImageHandoff(): FactoryImageHandoff | null {
  const file = cached === undefined ? load() : cached;
  if (
    file === null ||
    file.snapshotId === undefined ||
    file.commit === undefined ||
    file.fingerprint === undefined
  ) {
    return null;
  }
  return {
    commit: file.commit,
    fingerprint: file.fingerprint,
    snapshotId: file.snapshotId
  };
}

function load(): HandoffFile | null {
  try {
    const value: unknown = JSON.parse(readFileSync(HANDOFF_PATH, "utf8"));
    if (typeof value === "object" && value !== null) {
      cached = value as HandoffFile;
      return cached;
    }
  } catch {
    // No published image was recorded for this build.
  }
  cached = null;
  return null;
}
