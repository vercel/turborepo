export interface TurboIgnoreArgs {
  // the working directory to use when looking for a workspace
  directory?: string;
  // the workspace to check for changes
  workspace?: string;
  // A ref/head to compare against if no previously deployed SHA is available
  fallback?: string;
}
