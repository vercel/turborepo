/**
 * Entry point every factory image trigger goes through.
 *
 * Claiming the ledger and cancelling what it superseded happen here, in
 * one place, so the merge webhook and the operator button cannot diverge
 * on the "newest revision wins" rule.
 */

import { randomUUID } from "node:crypto";

import { factoryImageFingerprint } from "./factory-image";
import {
  claimFactoryImageProvisioning,
  claimFactoryImagePublication,
  claimFactoryImage,
  isFactoryImageRegistryConfigured,
  publishFactoryImage,
  readFactoryImageBuild,
  readFactoryImagePointer,
  readFactoryImageState,
  recordFactoryImageProgress
} from "./factory-image-registry";
import {
  type FactoryImagePointer,
  type FactoryImageTrigger,
  factoryImageSandboxName,
  isFactoryImageBuildActive,
  isStaleFactoryImageBuild
} from "./factory-image-types";
import {
  createFactorySandbox,
  deleteFactorySandbox,
  getFactorySandbox,
  pruneFactorySandboxes,
  readFactoryProgress,
  snapshotFactorySandbox,
  startFactoryProvisioning
} from "./factory-sandbox";

export interface TriggerFactoryImageInput {
  readonly commit: string;
  readonly ref: string;
  readonly trigger: FactoryImageTrigger;
}

export type TriggerFactoryImageResult =
  | {
      readonly buildId: string;
      /** Builds cancelled because this revision is newer. */
      readonly cancelled: readonly string[];
      readonly commit: string;
      readonly state: "claimed";
    }
  | {
      readonly buildId: string;
      readonly commit: string;
      readonly state: "in-progress";
    }
  | {
      readonly commit: string;
      readonly pointer: FactoryImagePointer;
      readonly state: "current";
    };

export async function triggerFactoryImageBuild(
  input: TriggerFactoryImageInput
): Promise<TriggerFactoryImageResult> {
  if (!isFactoryImageRegistryConfigured()) {
    throw new Error(
      "Factory image builds require a private Vercel Blob store."
    );
  }

  const buildId = randomUUID().replaceAll("-", "");
  const claim = await claimFactoryImage({
    buildId,
    commit: input.commit,
    fingerprint: factoryImageFingerprint(),
    now: new Date().toISOString(),
    ref: input.ref,
    sandboxName: factoryImageSandboxName(input.commit, buildId),
    trigger: input.trigger
  });

  if (claim.kind === "current") {
    return {
      commit: input.commit,
      pointer: claim.pointer,
      state: "current"
    };
  }
  if (claim.kind === "in-progress") {
    return {
      buildId: claim.build.id,
      commit: input.commit,
      state: "in-progress"
    };
  }

  // The ledger already marked these cancelled. Delete their sandboxes so a
  // superseded build cannot keep burning sandbox time.
  const cancelled: string[] = [];
  for (const superseded of claim.superseded) {
    try {
      await deleteFactorySandbox(superseded.sandboxName);
    } catch (error) {
      console.error(
        `Could not delete sandbox ${superseded.sandboxName}.`,
        error
      );
    }
    cancelled.push(superseded.id);
  }

  try {
    await startClaimedFactoryImageBuild(buildId);
  } catch (error) {
    await recordFactoryImageProgress(buildId, {
      finishedAt: new Date().toISOString(),
      message: error instanceof Error ? error.message : String(error),
      status: "failed"
    }).catch((failure: unknown) => {
      console.error("Could not record the factory image failure.", failure);
    });
    await deleteFactorySandbox(claim.build.sandboxName).catch(() => {});
    throw error;
  }

  return {
    buildId,
    cancelled,
    commit: input.commit,
    state: "claimed"
  };
}

async function startClaimedFactoryImageBuild(buildId: string): Promise<void> {
  const build = await claimFactoryImageProvisioning(buildId);
  if (build === null) return;

  const pointer = await readFactoryImagePointer();
  const baseSnapshotId =
    pointer !== null && pointer.fingerprint === factoryImageFingerprint()
      ? pointer.snapshotId
      : undefined;
  const sandbox =
    (await getFactorySandbox(build.sandboxName)) ??
    (await createFactorySandbox({
      baseSnapshotId,
      buildId: build.id,
      commit: build.commit,
      name: build.sandboxName
    }));

  const current = await readFactoryImageBuild(build.id);
  if (current === null || !isFactoryImageBuildActive(current)) {
    await deleteFactorySandbox(build.sandboxName);
    return;
  }

  await startFactoryProvisioning(sandbox, {
    revision: build.commit,
    warmBuild: true
  });
  await recordFactoryImageProgress(build.id, {
    message:
      baseSnapshotId === undefined
        ? "Building from the base image."
        : "Building from the previous factory image.",
    phase: "provisioning"
  });
}

export async function reconcileFactoryImageBuilds(): Promise<
  Readonly<Record<string, string>>
> {
  if (!isFactoryImageRegistryConfigured()) return {};

  const logs: Record<string, string> = {};
  for (const build of (await readFactoryImageState()).builds) {
    if (!isFactoryImageBuildActive(build)) continue;
    try {
      if (
        build.status === "queued" ||
        (build.status === "building" &&
          build.phase === "starting" &&
          isStaleFactoryImageBuild(build, new Date().toISOString()))
      ) {
        await startClaimedFactoryImageBuild(build.id);
        continue;
      }
      if (build.status === "building") {
        const log = await reconcileBuildingImage(build.id);
        if (log) logs[build.id] = log;
      }
      if (build.status === "publishing") await publishReadyImage(build.id);
    } catch (error) {
      console.error(
        `Could not reconcile factory image build ${build.id}.`,
        error
      );
    }
  }
  return logs;
}

async function reconcileBuildingImage(buildId: string): Promise<string | null> {
  const build = await readFactoryImageBuild(buildId);
  if (build === null || build.status !== "building") return null;
  const sandbox = await getFactorySandbox(build.sandboxName);
  if (sandbox === null) {
    if (build.phase === "starting") return null;
    await recordFactoryImageProgress(build.id, {
      finishedAt: new Date().toISOString(),
      message: "The build sandbox disappeared.",
      status: "failed"
    });
    return null;
  }

  const progress = await readFactoryProgress(sandbox);
  await recordFactoryImageProgress(build.id, {
    message:
      progress.warnings.length === 0
        ? undefined
        : `${progress.warnings.length} warning(s).`,
    phase: progress.phase
  });
  if (progress.exitCode === null) return progress.logTail;
  if (progress.exitCode !== 0) {
    const message = `Provisioning exited with code ${progress.exitCode} during phase "${progress.phase}".`;
    await recordFactoryImageProgress(build.id, {
      finishedAt: new Date().toISOString(),
      message,
      status: "failed"
    });
    console.error(message, progress.logTail);
    await deleteFactorySandbox(build.sandboxName);
    return progress.logTail;
  }
  await publishReadyImage(
    build.id,
    progress.warnings,
    progress.manifest ?? undefined
  );
  return progress.logTail;
}

async function publishReadyImage(
  buildId: string,
  warnings?: readonly string[],
  manifest?: Readonly<Record<string, string>>
): Promise<void> {
  const build = await claimFactoryImagePublication(buildId);
  if (build === null) return;
  const sandbox = await getFactorySandbox(build.sandboxName);
  if (sandbox === null) {
    await recordFactoryImageProgress(build.id, {
      finishedAt: new Date().toISOString(),
      message: "The build sandbox disappeared before the snapshot.",
      status: "failed"
    });
    return;
  }
  const snapshotId = await snapshotFactorySandbox(sandbox);
  const pointer = await publishFactoryImage(build.id, {
    snapshotId,
    tools: manifest,
    warmBuild: true,
    warnings
  });
  if (pointer !== null) await pruneFactorySandboxes([pointer.sandboxName]);
}
