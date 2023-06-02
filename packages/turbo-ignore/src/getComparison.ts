import { info } from "./logger";
import { TurboIgnoreArgs } from "./types";

export interface GetComparisonArgs extends TurboIgnoreArgs {
  // the workspace to check for changes
  workspace: string;
}

export function getComparison(args: GetComparisonArgs): {
  ref: string;
  type: "previousDeploy" | "headRelative" | "customFallback" | "specifiedManually";
} | null {
  const { fallback, since, workspace } = args;

  if (since) {
    // if user has provided a commit to compare to, we use it in the first place
    return { ref: since, type: 'specifiedManually' };
  }

  if (process.env.VERCEL === "1") {
    if (process.env.VERCEL_GIT_PREVIOUS_SHA) {
      // use the commit SHA of the last successful deployment for this project / branch
      info(
        `Found previous deployment ("${process.env.VERCEL_GIT_PREVIOUS_SHA}") for "${workspace}" on branch "${process.env.VERCEL_GIT_COMMIT_REF}"`
      );
      return {
        ref: process.env.VERCEL_GIT_PREVIOUS_SHA,
        type: "previousDeploy",
      };
    } else {
      info(
        `No previous deployments found for "${workspace}" on branch "${process.env.VERCEL_GIT_COMMIT_REF}".`
      );
      if (fallback) {
        info(`Falling back to ref ${fallback}`);
        return { ref: fallback, type: "customFallback" };
      }

      return null;
    }
  }
  return { ref: "HEAD^", type: "headRelative" };
}
