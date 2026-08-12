export const MAINTENANCE_RUN_ACTION = "run-daily-maintenance";
export const PERFORMANCE_RUN_ACTION = "run-daily-performance";

// Sent as `x-operator-action` by the dashboard and required by the operator
// channel, so both sides of a trigger stay in sync.
export type OperatorRunAction =
  | typeof MAINTENANCE_RUN_ACTION
  | typeof PERFORMANCE_RUN_ACTION;
