/**
 * Build-time handoff of the published factory image to the Eve sandbox
 * backend.
 *
 * Eve resolves `revalidationKey` asynchronously while it compiles, then
 * asks the sandbox definition for its `backend` synchronously when it
 * prewarms the template — possibly from a second process in the same
 * build. Reading the pointer needs `await`, so the async hook records it
 * here and the synchronous factory picks it up.
 *
 * This is only ever an optimization. Without the file the template is
 * built from the base image instead of the published snapshot: slower,
 * identical result, because every provisioning phase is idempotent. The
 * template's identity comes from `revalidationKey`, never from this file.
 */

import { readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";

export interface FactoryImageHandoff {
  readonly commit: string;
  readonly snapshotId: string;
}

interface HandoffFile {
  readonly commit?: string;
  readonly fingerprint: string;
  readonly snapshotId?: string;
}

const HANDOFF_PATH = path.join(tmpdir(), "turborepo-factory-image-base.json");

let cached: HandoffFile | null | undefined;

export function writeFactoryImageHandoff(handoff: {
  readonly commit?: string;
  readonly fingerprint: string;
  readonly snapshotId?: string;
}): void {
  cached = handoff;
  try {
    writeFileSync(HANDOFF_PATH, JSON.stringify(handoff), "utf8");
  } catch (error) {
    console.warn("Could not record the factory image base snapshot.", error);
  }
}

/**
 * Base snapshot recorded for the current toolchain, or `null` when there
 * is none (or when the recorded one belongs to a different toolchain).
 */
export function readFactoryImageHandoff(
  fingerprint: string
): FactoryImageHandoff | null {
  const file = cached === undefined ? load() : cached;
  if (
    file === null ||
    file.fingerprint !== fingerprint ||
    file.snapshotId === undefined ||
    file.commit === undefined
  ) {
    return null;
  }
  return { commit: file.commit, snapshotId: file.snapshotId };
}

function load(): HandoffFile | null {
  try {
    const value: unknown = JSON.parse(readFileSync(HANDOFF_PATH, "utf8"));
    if (
      typeof value === "object" &&
      value !== null &&
      typeof (value as HandoffFile).fingerprint === "string"
    ) {
      cached = value as HandoffFile;
      return cached;
    }
  } catch {
    // No handoff was recorded in this build; provision from the base image.
  }
  cached = null;
  return null;
}
