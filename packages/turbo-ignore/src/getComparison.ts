import { info } from "./logger";

export function getComparison({
  workspace,
  fallback = true,
}: {
  workspace: string;
  fallback?: boolean;
}): {
  ref: string;
  type: "previousDeploy" | "headRelative";
} | null {
  if (process.env.VERCEL === "1") {
    if (process.env.VERCEL_GIT_PREVIOUS_SHA) {
      // use the commit SHA of the last successful deployment for this project / branch
      return {
        ref: process.env.VERCEL_GIT_PREVIOUS_SHA,
        type: "previousDeploy",
      };
    } else {
      info(
        `no previous deployments found for "${workspace}" on "${process.env.VERCEL_GIT_COMMIT_REF}".`
      );
      if (fallback) {
        info(`falling back to HEAD^`);
        return { ref: "HEAD^", type: "headRelative" };
      }

      return null;
    }
  }
  return { ref: "HEAD^", type: "headRelative" };
}
