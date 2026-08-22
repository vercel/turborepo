/**
 * Entry point every factory image trigger goes through.
 *
 * Claiming the ledger and cancelling what it superseded happen here, in
 * one place, so the merge webhook and the operator button cannot diverge
 * on the "newest revision wins" rule.
 */

import { randomUUID } from "node:crypto";

import { getRun, start } from "workflow/api";

import { factoryImageWorkflow } from "../../workflows/factory-image";
import { factoryImageFingerprint } from "./factory-image";
import {
  claimFactoryImage,
  isFactoryImageRegistryConfigured,
  recordFactoryImageProgress
} from "./factory-image-registry";
import {
  type FactoryImagePointer,
  type FactoryImageTrigger,
  factoryImageSandboxName
} from "./factory-image-types";
import { deleteFactorySandbox } from "./factory-sandbox";

export interface TriggerFactoryImageInput {
  readonly commit: string;
  readonly ref: string;
  readonly trigger: FactoryImageTrigger;
  /** Compile `turbo` once inside the image. On by default. */
  readonly warmBuild?: boolean;
}

export type TriggerFactoryImageResult =
  | {
      readonly buildId: string;
      /** Builds cancelled because this revision is newer. */
      readonly cancelled: readonly string[];
      readonly commit: string;
      readonly state: "claimed";
      readonly workflowRunId: string;
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

  // The ledger already marked these cancelled. Stop their workflow runs
  // and delete their sandboxes so a superseded build cannot keep burning
  // sandbox time while the newest revision builds.
  const cancelled: string[] = [];
  for (const superseded of claim.superseded) {
    if (superseded.workflowRunId !== undefined) {
      try {
        await getRun(superseded.workflowRunId).cancel({
          cancelReason: `Superseded by ${input.commit.slice(0, 7)}.`
        });
      } catch (error) {
        console.error(
          `Could not cancel workflow run ${superseded.workflowRunId}.`,
          error
        );
      }
    }
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

  let run;
  try {
    run = await start(factoryImageWorkflow, [
      {
        buildId,
        commit: input.commit,
        ref: input.ref,
        sandboxName: claim.build.sandboxName,
        warmBuild: input.warmBuild ?? true
      }
    ]);
  } catch (error) {
    // Nothing will ever report progress for this build, so retire it
    // rather than leaving a claim that deduplicates later deliveries.
    await recordFactoryImageProgress(buildId, {
      finishedAt: new Date().toISOString(),
      message: "The build workflow could not be started.",
      status: "failed"
    }).catch((failure: unknown) => {
      console.error("Could not retire the factory image build.", failure);
    });
    throw error;
  }
  // Recorded here as well as inside the workflow's first step so a merge
  // that lands moments later already has a run id to cancel.
  try {
    await recordFactoryImageProgress(buildId, { workflowRunId: run.runId });
  } catch (error) {
    console.error("Could not record the factory image workflow run.", error);
  }

  return {
    buildId,
    cancelled,
    commit: input.commit,
    state: "claimed",
    workflowRunId: run.runId
  };
}
