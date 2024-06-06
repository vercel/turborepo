import { type TurboIgnoreTelemetry } from "@turbo/telemetry";

export type NonFatalErrorKey =
  | "MISSING_LOCKFILE"
  | "NO_PACKAGE_MANAGER"
  | "UNREACHABLE_PARENT"
  | "INVALID_COMPARISON";

export interface NonFatalError {
  regex: Array<RegExp>;
  message: string;
}

export type NonFatalErrors = Record<NonFatalErrorKey, NonFatalError>;

export type TurboIgnoreArg = string | undefined;

export interface TurboIgnoreOptions {
  // the working directory to use when looking for a workspace
  directory?: string;
  // the workspace to check for changes
  workspace?: string;
  // the task to run, if not build
  task?: string;
  // A ref/head to compare against if no previously deployed SHA is available
  fallback?: string;
  // An explicit version of turbo to use
  turboVersion?: string;
  // The maxBuffer for the child process in KB
  maxBuffer?: number;
  // The telemetry client
  telemetry?: TurboIgnoreTelemetry;
}
