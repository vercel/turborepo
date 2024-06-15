import { execSync } from "node:child_process";
import { info } from "./logger";
import type { TurboIgnoreOptions } from "./types";

export interface GetComparisonArgs extends TurboIgnoreOptions {
  // the workspace to check for changes
  workspace: string;
  // A ref/head to compare against if no previously deployed SHA is available
  fallback?: string;
}

export function validateSHAExists(ref: string): boolean {
  try {
    execSync(`git cat-file -t ${ref}`, { stdio: "ignore" });
    return true;
  } catch (e) {
    return false;
  }
}

export function fallback(
  args: GetComparisonArgs
): { ref: string; type: "customFallback" } | null {
  if (args.fallback) {
    info(`Falling back to ref ${args.fallback}`);
    return { ref: args.fallback, type: "customFallback" };
  }

  return null;
}

export function getComparison(args: GetComparisonArgs): {
  ref: string;
  type: "previousDeploy" | "headRelative" | "customFallback";
} | null {
  const { workspace } = args;
  if (process.env.VERCEL === "1") {
    if (process.env.VERCEL_GIT_PREVIOUS_SHA) {
      if (validateSHAExists(process.env.VERCEL_GIT_PREVIOUS_SHA)) {
        // use the commit SHA of the last successful deployment for this project / branch
        info(
          `Found previous deployment ("${
            process.env.VERCEL_GIT_PREVIOUS_SHA
          }") for "${workspace}"${
            process.env.VERCEL_GIT_COMMIT_REF
              ? ` on branch "${process.env.VERCEL_GIT_COMMIT_REF}"`
              : ""
          }`
        );
        return {
          ref: process.env.VERCEL_GIT_PREVIOUS_SHA,
          type: "previousDeploy",
        };
      }

      // if the previous deployment is unreachable, use the fallback
      info(
        `Previous deployment ("${
          process.env.VERCEL_GIT_PREVIOUS_SHA
        }") for "${workspace}"${
          process.env.VERCEL_GIT_COMMIT_REF
            ? ` on branch "${process.env.VERCEL_GIT_COMMIT_REF}"`
            : ""
        } is unreachable.`
      );
      return fallback(args);
    }

    info(
      `No previous deployments found for "${workspace}"${
        process.env.VERCEL_GIT_COMMIT_REF
          ? ` on branch "${process.env.VERCEL_GIT_COMMIT_REF}"`
          : ""
      }`
    );
    return fallback(args);
  } else if (args.fallback) {
    return fallback(args);
  }
  return { ref: "HEAD^", type: "headRelative" };
}
