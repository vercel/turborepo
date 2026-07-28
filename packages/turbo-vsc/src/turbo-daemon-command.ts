export type TurboDaemonCommand = "start" | "stop" | "status";

export function createTurboDaemonArgs(command: TurboDaemonCommand): string[] {
  return ["daemon", command];
}
