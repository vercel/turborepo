export type NonFatalErrorKey =
  | "MISSING_LOCKFILE"
  | "NO_PACKAGE_MANAGER"
  | "UNREACHABLE_PARENT"
  | "UNREACHABLE_COMMIT";

export interface NonFatalError {
  regex: RegExp;
  message: string;
}

export type NonFatalErrors = Record<NonFatalErrorKey, NonFatalError>;

export interface TurboIgnoreArgs {
  // the working directory to use when looking for a workspace
  directory?: string;
  // the workspace to check for changes
  workspace?: string;
  // the task to run, if not build
  task?: string;
  // A ref/head to compare against if no previously deployed SHA is available
  fallback?: string;
}
