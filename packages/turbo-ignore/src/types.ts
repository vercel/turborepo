export interface TurboIgnoreArgs {
  // the workspace to check for changes
  workspace: string | null;
  // if the previous commit should be used to compare against when no previously deployed SHA is available
  filterFallback: boolean;
}
