export interface TurboTaskFinding {
  name: string;
  persistent: boolean;
  cache: boolean | null;
  scriptExists: boolean;
  shouldValidate: boolean;
}

const longRunningTaskNames = new Set(["dev", "start", "serve", "preview"]);

export function auditExampleTasks(
  packageJson: Record<string, unknown>,
  turboJson: Record<string, unknown>
): {
  turboTasks: TurboTaskFinding[];
  scriptOnlyValidationTasks: string[];
  recommendedTurboTasksToRun: string[];
} {
  const scripts = asObject(packageJson.scripts) ?? {};
  const tasks = collectTasks(turboJson);
  const turboTasks = Object.entries(tasks)
    .map(([name, config]): TurboTaskFinding => {
      const taskConfig = asObject(config) ?? {};
      const persistent = taskConfig.persistent === true;
      return {
        name,
        persistent,
        cache: typeof taskConfig.cache === "boolean" ? taskConfig.cache : null,
        scriptExists: typeof scripts[name] === "string",
        shouldValidate: !persistent && !longRunningTaskNames.has(name)
      };
    })
    .sort((a, b) => a.name.localeCompare(b.name));

  return {
    turboTasks,
    scriptOnlyValidationTasks: Object.keys(scripts)
      .filter((name) => !tasks[name] && !longRunningTaskNames.has(name))
      .sort(),
    recommendedTurboTasksToRun: turboTasks
      .filter((task) => task.shouldValidate)
      .map((task) => task.name)
  };
}

function collectTasks(
  turboJson: Record<string, unknown>
): Record<string, unknown> {
  return asObject(turboJson.tasks) ?? asObject(turboJson.pipeline) ?? {};
}

function asObject(value: unknown): Record<string, unknown> | null {
  return typeof value === "object" && value !== null && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}
