import type { PipelineV1, SchemaV1 } from "@turbo/types";

/** This utility allows us to check that the "pipeline" key exists,
 *  and early exit if it does not.
 *
 *  Codemods for v1 assume that the "pipeline" key is present.
 *  However, this isn't a safe assumption.
 *  A user could run the codemod that changes the "pipeline" key to "tasks"
 *  and end up failing on a codemod that comes later. Later attempts to run the codemods would fail.
 *
 *  See https://github.com/vercel/turborepo/issues/8495. */
export const isPipelineKeyMissing = (config: SchemaV1) => {
  return !config.pipeline as unknown as PipelineV1 | undefined;
};
