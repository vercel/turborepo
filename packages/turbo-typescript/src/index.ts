export type {
  CommandName,
  Diagnostic,
  ProjectReferencesOptions,
  ProjectReferencesResult
} from "./types";
export { ProjectReferencesError } from "./types";
export {
  initProjectReferences,
  checkProjectReferences,
  writeProjectReferences,
  getProjectReferenceCandidates
} from "./sync";
