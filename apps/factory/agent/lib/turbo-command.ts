export type PackageManager = "bun" | "npm" | "pnpm" | "yarn";

export interface TurboRunCommand {
  command: PackageManager | "bunx";
  args: string[];
  tasks: string[];
}

export function buildTurboRunCommand(
  packageManager: PackageManager,
  taskNames: readonly string[]
): TurboRunCommand {
  const tasks = [...new Set(taskNames)];
  if (tasks.length === 0) {
    throw new Error("At least one Turbo task is required.");
  }
  for (const task of tasks) {
    if (task.trim() === "" || task.startsWith("-")) {
      throw new Error(`Invalid Turbo task name: ${JSON.stringify(task)}.`);
    }
  }

  const turboArgs = ["run", ...tasks, "--continue=always"];
  switch (packageManager) {
    case "bun": {
      return { command: "bunx", args: ["turbo", ...turboArgs], tasks };
    }
    case "npm": {
      return {
        command: "npm",
        args: ["exec", "turbo", "--", ...turboArgs],
        tasks
      };
    }
    case "pnpm":
    case "yarn": {
      return {
        command: packageManager,
        args: ["exec", "turbo", ...turboArgs],
        tasks
      };
    }
  }
}
