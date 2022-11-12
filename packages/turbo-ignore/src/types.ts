export interface TurboIgnoreArgs {
  // the working directory to use when looking for a workspace
  directory?: string;
  // the workspace to check for changes
  workspace: string | null;
  // if the previous commit should be used to compare against when no previously deployed SHA is available
  filterFallback: boolean;
}
