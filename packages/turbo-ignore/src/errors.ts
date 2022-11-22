import { NonFatalErrorKeys, NonFatalErrors } from "./types";

export const NON_FATAL_ERRORS: NonFatalErrors = {
  MISSING_LOCKFILE: {
    regex:
      /reading (yarn.lock|package-lock.json|pnpm-lock.yaml):.*?no such file or directory/,
    message: `turbo-ignore could not complete - no lockfile found, please commit one to your repository`,
  },
  NO_PACKAGE_MANAGER: {
    regex:
      /run failed: We did not detect an in-use package manager for your project/,
    message: `turbo-ignore could not complete - no package manager detected, please commit a lockfile, or set "packageManager" in your root "package.json"`,
  },
  FIRST_COMMIT: {
    regex: /failed to resolve packages to run: commit HEAD\^ does not exist/,
    message: `turbo-ignore could not complete - not enough information available to compare`,
  },
};

export function shouldWarn({ err }: { err: string }): {
  level: "warn" | "error";
  message: string;
  code: NonFatalErrorKeys | "UNKNOWN_ERROR";
} {
  const knownError = Object.keys(NON_FATAL_ERRORS).find((key) => {
    const { regex } = NON_FATAL_ERRORS[key as NonFatalErrorKeys];
    return regex.test(err);
  });

  if (knownError) {
    return {
      level: "warn",
      message: NON_FATAL_ERRORS[knownError as NonFatalErrorKeys].message,
      code: knownError as NonFatalErrorKeys,
    };
  }

  return { level: "error", message: err, code: "UNKNOWN_ERROR" };
}
