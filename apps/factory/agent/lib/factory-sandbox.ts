/**
 * Vercel Sandbox operations behind a factory image build.
 *
 * The build sandbox is a plain `@vercel/sandbox` VM (not an Eve session):
 * the workflow creates it, detaches the provisioning script, polls the
 * markers it writes, and snapshots the result. Every helper is
 * name-addressed so a workflow step can reattach after a suspension
 * without serializing a live handle.
 */

import { APIError, Sandbox } from "@vercel/sandbox";

import {
  type FactoryImageOptions,
  type FactoryImageProgress,
  FACTORY_IMAGE_BASE,
  FACTORY_IMAGE_SCRIPT_FILE,
  factoryImageProgressCommand,
  factoryImageScript,
  factoryImageStartCommand,
  parseFactoryImageProgress
} from "./factory-image";
import { FACTORY_IMAGE_SANDBOX_PREFIX } from "./factory-image-types";

/** Hard cap on one build. Provisioning from scratch takes ~20 minutes. */
const BUILD_TIMEOUT_MS = 60 * 60 * 1000;
const BUILD_VCPUS = 8;
/** Build sandboxes kept for inspection before older ones are deleted. */
const KEEP_BUILD_SANDBOXES = 4;

export interface CreateFactorySandboxInput {
  /**
   * Snapshot to start from. Passing the previous image makes a merge
   * build incremental: the toolchain is already installed, so only the
   * checkout, dependencies, and warm build have to catch up.
   */
  readonly baseSnapshotId?: string;
  readonly buildId: string;
  readonly commit: string;
  readonly name: string;
}

export async function createFactorySandbox(
  input: CreateFactorySandboxInput
): Promise<Sandbox> {
  const shared = {
    name: input.name,
    resources: { vcpus: BUILD_VCPUS },
    tags: {
      build: input.buildId.slice(0, 8),
      commit: input.commit.slice(0, 12),
      role: "factory-image"
    },
    timeout: BUILD_TIMEOUT_MS
  } as const;
  return input.baseSnapshotId === undefined
    ? Sandbox.create({ ...shared, image: FACTORY_IMAGE_BASE })
    : Sandbox.create({
        ...shared,
        source: { snapshotId: input.baseSnapshotId, type: "snapshot" }
      });
}

export async function getFactorySandbox(name: string): Promise<Sandbox | null> {
  try {
    return await Sandbox.get({ name });
  } catch (error) {
    if (isSandboxMissing(error)) return null;
    throw error;
  }
}

/**
 * Writes the provisioning script and starts it detached, returning once
 * the script is running rather than once it is finished.
 */
export async function startFactoryProvisioning(
  sandbox: Sandbox,
  options: FactoryImageOptions
): Promise<void> {
  await sandbox.writeFiles([
    {
      content: Buffer.from(factoryImageScript(options), "utf8"),
      path: FACTORY_IMAGE_SCRIPT_FILE
    }
  ]);
  const started = await sandbox.runCommand({
    args: ["-lc", factoryImageStartCommand()],
    cmd: "bash",
    detached: true
  });
  const finished = await started.wait();
  if (finished.exitCode !== 0) {
    throw new Error(
      `Could not start factory image provisioning: ${await finished.stderr()}`
    );
  }
}

export async function readFactoryProgress(
  sandbox: Sandbox
): Promise<FactoryImageProgress> {
  const result = await sandbox.runCommand({
    args: ["-lc", factoryImageProgressCommand()],
    cmd: "bash"
  });
  return parseFactoryImageProgress(await result.stdout());
}

/**
 * Captures the image. Snapshotting stops the sandbox, so the build is
 * finished by the time this resolves.
 */
export async function snapshotFactorySandbox(
  sandbox: Sandbox
): Promise<string> {
  const snapshot = await sandbox.snapshot();
  return snapshot.snapshotId;
}

export async function deleteFactorySandbox(name: string): Promise<void> {
  const sandbox = await getFactorySandbox(name);
  if (sandbox === null) return;
  try {
    await sandbox.delete();
  } catch (error) {
    if (!isSandboxMissing(error)) throw error;
  }
}

/**
 * Deletes build sandboxes that no longer matter, newest kept first.
 *
 * Only sandboxes are removed. Snapshots are left alone: an Eve template
 * built on top of a published image is a descendant of that snapshot, so
 * deleting one could break a deployment that is still serving traffic.
 */
export async function pruneFactorySandboxes(
  protectedNames: readonly string[]
): Promise<string[]> {
  const listed = await Sandbox.list({
    limit: 50,
    namePrefix: FACTORY_IMAGE_SANDBOX_PREFIX,
    sortBy: "createdAt",
    sortOrder: "desc"
  });
  const sandboxes = await listed.toArray();
  const keep = new Set(protectedNames);
  const deleted: string[] = [];
  for (const sandbox of sandboxes.slice(KEEP_BUILD_SANDBOXES)) {
    if (keep.has(sandbox.name)) continue;
    try {
      await deleteFactorySandbox(sandbox.name);
      deleted.push(sandbox.name);
    } catch (error) {
      console.error(`Could not delete sandbox ${sandbox.name}.`, error);
    }
  }
  return deleted;
}

function isSandboxMissing(error: unknown): boolean {
  return error instanceof APIError && error.response.status === 404;
}
