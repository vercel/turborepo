export interface TurboRunTerminalOptions {
  name: string;
  shellPath: string;
  shellArgs: string[];
  isTransient: true;
}

export function createTurboRunTerminalOptions(
  turboPath: string,
  taskName: string
): TurboRunTerminalOptions {
  return {
    name: taskName,
    shellPath: turboPath,
    shellArgs: ["run", taskName],
    isTransient: true
  };
}
