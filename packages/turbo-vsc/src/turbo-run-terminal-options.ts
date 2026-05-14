const TASK_NAME_PATTERN = /^(?!(?:-|$))[A-Za-z0-9_:@./#-]+$/;

export interface TurboRunTerminalOptions {
  name: string;
  shellPath: string;
  shellArgs: string[];
  isTransient: true;
}

export function sanitizeTurboRunTaskName(
  taskName: unknown
): string | undefined {
  if (typeof taskName !== "string") {
    return undefined;
  }

  if (!TASK_NAME_PATTERN.test(taskName)) {
    return undefined;
  }

  return taskName;
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
