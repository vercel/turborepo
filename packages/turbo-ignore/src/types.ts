export type NonFatalErrorKeys =
  | "MISSING_LOCKFILE"
  | "NO_PACKAGE_MANAGER"
  | "FIRST_COMMIT";

export interface NonFatalError {
  regex: RegExp;
  message: string;
}

export type NonFatalErrors = Record<NonFatalErrorKeys, NonFatalError>;

export interface TurboIgnoreArgs {
  // the working directory to use when looking for a workspace
  directory?: string;
  // the workspace to check for changes
  workspace?: string;
  // A ref/head to compare against if no previously deployed SHA is available
  fallback?: string;
}
