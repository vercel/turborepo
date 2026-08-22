import { getWorkflowMetadata, sleep } from "workflow";

/**
 * Builds one factory image and publishes it as the snapshot every
 * Turborepo agent boots from.
 *
 * Provisioning runs detached inside the sandbox, so each step here is
 * short: start, poll, publish. Every step re-reads the ledger first and
 * gives up the moment a newer merge has claimed it, which is what keeps
 * a burst of merges from producing more than one image.
 */

export interface FactoryImageWorkflowInput {
  readonly buildId: string;
  readonly commit: string;
  readonly ref: string;
  readonly sandboxName: string;
  readonly warmBuild: boolean;
}

export interface FactoryImageWorkflowResult {
  readonly buildId: string;
  readonly commit: string;
  readonly detail?: string;
  readonly snapshotId?: string;
  readonly status: "cancelled" | "failed" | "published";
}

type BeginResult = {
  readonly detail?: string;
  readonly state: "building" | "cancelled" | "failed";
};

type PollResult = {
  readonly detail?: string;
  readonly manifest?: Readonly<Record<string, string>>;
  readonly state: "building" | "cancelled" | "failed" | "ready";
  readonly warnings?: readonly string[];
};

const POLL_INTERVAL = "30s";
/** 30s × 80 ≈ 40 minutes, inside the build sandbox's own timeout. */
const MAX_POLLS = 80;

export async function factoryImageWorkflow(
  input: FactoryImageWorkflowInput
): Promise<FactoryImageWorkflowResult> {
  "use workflow";

  const { workflowRunId } = getWorkflowMetadata();
  const begun = await beginFactoryImageBuild(input, workflowRunId);
  if (begun.state !== "building") {
    return {
      buildId: input.buildId,
      commit: input.commit,
      detail: begun.detail,
      status: begun.state
    };
  }

  for (let poll = 0; poll < MAX_POLLS; poll += 1) {
    await sleep(POLL_INTERVAL);
    const progress = await pollFactoryImageBuild(input);
    if (progress.state === "building") continue;
    if (progress.state === "ready") {
      return await publishFactoryImageBuild(
        input,
        progress.warnings ?? [],
        progress.manifest
      );
    }
    return {
      buildId: input.buildId,
      commit: input.commit,
      detail: progress.detail,
      status: progress.state
    };
  }

  return await abandonFactoryImageBuild(
    input,
    `The build did not finish within ${MAX_POLLS} polls.`
  );
}

async function beginFactoryImageBuild(
  input: FactoryImageWorkflowInput,
  workflowRunId: string
): Promise<BeginResult> {
  "use step";

  const [
    { createFactorySandbox, deleteFactorySandbox, startFactoryProvisioning },
    registry,
    { factoryImageFingerprint }
  ] = await Promise.all([
    import("../agent/lib/factory-sandbox"),
    import("../agent/lib/factory-image-registry"),
    import("../agent/lib/factory-image")
  ]);

  const claimed = await registry.recordFactoryImageProgress(input.buildId, {
    phase: "starting",
    status: "building",
    workflowRunId
  });
  if (claimed === null || claimed.status !== "building") {
    await deleteFactorySandbox(input.sandboxName);
    return {
      detail: "Superseded before the build started.",
      state: "cancelled"
    };
  }

  try {
    // Booting from the previous image keeps a merge build incremental:
    // the toolchain is already in place, so only the checkout, workspace
    // dependencies, and warm build have to catch up.
    const pointer = await registry.readFactoryImagePointer();
    const baseSnapshotId =
      pointer !== null && pointer.fingerprint === factoryImageFingerprint()
        ? pointer.snapshotId
        : undefined;
    const sandbox = await createFactorySandbox({
      baseSnapshotId,
      buildId: input.buildId,
      commit: input.commit,
      name: input.sandboxName
    });
    await startFactoryProvisioning(sandbox, {
      revision: input.commit,
      warmBuild: input.warmBuild
    });
    await registry.recordFactoryImageProgress(input.buildId, {
      message:
        baseSnapshotId === undefined
          ? "Building from the base image."
          : "Building from the previous factory image.",
      phase: "provisioning"
    });
    return { state: "building" };
  } catch (error) {
    const detail = error instanceof Error ? error.message : String(error);
    await registry.recordFactoryImageProgress(input.buildId, {
      finishedAt: new Date().toISOString(),
      message: detail,
      status: "failed"
    });
    await deleteFactorySandbox(input.sandboxName);
    return { detail, state: "failed" };
  }
}

async function pollFactoryImageBuild(
  input: FactoryImageWorkflowInput
): Promise<PollResult> {
  "use step";

  const [
    { deleteFactorySandbox, getFactorySandbox, readFactoryProgress },
    registry
  ] = await Promise.all([
    import("../agent/lib/factory-sandbox"),
    import("../agent/lib/factory-image-registry")
  ]);

  const build = await registry.readFactoryImageBuild(input.buildId);
  if (build === null || build.status !== "building") {
    await deleteFactorySandbox(input.sandboxName);
    return { detail: "Superseded by a newer revision.", state: "cancelled" };
  }

  const sandbox = await getFactorySandbox(input.sandboxName);
  if (sandbox === null) {
    const detail = "The build sandbox disappeared.";
    await registry.recordFactoryImageProgress(input.buildId, {
      finishedAt: new Date().toISOString(),
      message: detail,
      status: "failed"
    });
    return { detail, state: "failed" };
  }

  const progress = await readFactoryProgress(sandbox);
  await registry.recordFactoryImageProgress(input.buildId, {
    message:
      progress.warnings.length === 0
        ? undefined
        : `${progress.warnings.length} warning(s).`,
    phase: progress.phase
  });

  if (progress.exitCode === null) return { state: "building" };
  if (progress.exitCode === 0) {
    return {
      manifest: progress.manifest ?? undefined,
      state: "ready",
      warnings: progress.warnings
    };
  }

  const detail = `Provisioning exited with code ${progress.exitCode} during phase "${progress.phase}".`;
  await registry.recordFactoryImageProgress(input.buildId, {
    finishedAt: new Date().toISOString(),
    message: detail,
    status: "failed"
  });
  console.error(detail, progress.logTail);
  await deleteFactorySandbox(input.sandboxName);
  return { detail, state: "failed" };
}

async function publishFactoryImageBuild(
  input: FactoryImageWorkflowInput,
  warnings: readonly string[],
  manifest: Readonly<Record<string, string>> | undefined
): Promise<FactoryImageWorkflowResult> {
  "use step";

  const [
    {
      deleteFactorySandbox,
      getFactorySandbox,
      pruneFactorySandboxes,
      snapshotFactorySandbox
    },
    registry
  ] = await Promise.all([
    import("../agent/lib/factory-sandbox"),
    import("../agent/lib/factory-image-registry")
  ]);

  const build = await registry.readFactoryImageBuild(input.buildId);
  if (build === null || build.status !== "building") {
    await deleteFactorySandbox(input.sandboxName);
    return {
      buildId: input.buildId,
      commit: input.commit,
      detail: "Superseded by a newer revision.",
      status: "cancelled"
    };
  }
  await registry.recordFactoryImageProgress(input.buildId, {
    phase: "snapshotting",
    status: "publishing"
  });

  const sandbox = await getFactorySandbox(input.sandboxName);
  if (sandbox === null) {
    const detail = "The build sandbox disappeared before the snapshot.";
    await registry.recordFactoryImageProgress(input.buildId, {
      finishedAt: new Date().toISOString(),
      message: detail,
      status: "failed"
    });
    return {
      buildId: input.buildId,
      commit: input.commit,
      detail,
      status: "failed"
    };
  }

  const snapshotId = await snapshotFactorySandbox(sandbox);
  const pointer = await registry.publishFactoryImage(input.buildId, {
    snapshotId,
    tools: manifest,
    warmBuild: input.warmBuild,
    warnings
  });
  if (pointer === null) {
    return {
      buildId: input.buildId,
      commit: input.commit,
      detail: "A newer revision published first.",
      snapshotId,
      status: "cancelled"
    };
  }

  await pruneFactorySandboxes([pointer.sandboxName]);
  return {
    buildId: input.buildId,
    commit: input.commit,
    snapshotId,
    status: "published"
  };
}

async function abandonFactoryImageBuild(
  input: FactoryImageWorkflowInput,
  detail: string
): Promise<FactoryImageWorkflowResult> {
  "use step";

  const [{ deleteFactorySandbox }, registry] = await Promise.all([
    import("../agent/lib/factory-sandbox"),
    import("../agent/lib/factory-image-registry")
  ]);
  await registry.recordFactoryImageProgress(input.buildId, {
    finishedAt: new Date().toISOString(),
    message: detail,
    status: "failed"
  });
  await deleteFactorySandbox(input.sandboxName);
  return {
    buildId: input.buildId,
    commit: input.commit,
    detail,
    status: "failed"
  };
}
