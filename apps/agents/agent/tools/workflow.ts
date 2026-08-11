// Enables the `Workflow` tool so example updates fan out into one child run
// per stale example instead of one run that tries to update everything.
export { ExperimentalWorkflow as default } from "eve/tools";
