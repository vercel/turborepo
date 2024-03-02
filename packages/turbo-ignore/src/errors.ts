import type { NonFatalErrorKey, NonFatalErrors } from "./types";

export const NON_FATAL_ERRORS: NonFatalErrors = {
  MISSING_LOCKFILE: {
    regex: [
      /reading (?:yarn.lock|package-lock.json|pnpm-lock.yaml):.*?no such file or directory/i,
    ],
    message: `turbo-ignore could not complete - no lockfile found, please commit one to your repository`,
  },
  NO_PACKAGE_MANAGER: {
    regex: [
      /run failed: We did not detect an in-use package manager for your project/i,
    ],
    message: `turbo-ignore could not complete - no package manager detected, please commit a lockfile, or set "packageManager" in your root "package.json"`,
  },
  UNREACHABLE_PARENT: {
    regex: [/failed to resolve packages to run: commit HEAD\^ does not exist/i],
    message: `turbo-ignore could not complete - parent commit does not exist or is unreachable`,
  },
  INVALID_COMPARISON: {
    regex: [
      /commit \S+ does not exist/i,
      /unknown revision/i,
      /invalid symmetric difference expression/i,
    ],
    message: `turbo-ignore could not complete - a ref or SHA is invalid. It could have been removed from the branch history via a force push, or this could be a shallow clone with insufficient history`,
  },
};

export function shouldWarn({ err }: { err: string }): {
  level: "warn" | "error";
  message: string;
  code: NonFatalErrorKey | "UNKNOWN_ERROR";
} {
  const knownError = Object.keys(NON_FATAL_ERRORS).find((key) => {
    const { regex } = NON_FATAL_ERRORS[key as NonFatalErrorKey];
    return regex.some((r) => r.test(err));
  });

  if (knownError) {
    return {
      level: "warn",
      message: NON_FATAL_ERRORS[knownError as NonFatalErrorKey].message,
      code: knownError as NonFatalErrorKey,
    };
  }

  return { level: "error", message: err, code: "UNKNOWN_ERROR" };
}
