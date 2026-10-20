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
      const cache =
        typeof taskConfig.cache === "boolean" ? taskConfig.cache : null;
      return {
        name,
        persistent,
        cache,
        scriptExists: typeof scripts[name] === "string",
        // Only treat a task as a mandatory pass/fail validation target when it
        // can actually run to completion in an ephemeral sandbox. Persistent
        // and long-running server tasks never terminate, and tasks that opt
        // out of Turbo caching (`cache: false`) are declaring themselves
        // non-deterministic or side-effecting: database migrations/seeds
        // (e.g. `db:migrate:deploy`, `db:push`, `db:seed`), destructive tasks
        // (`clean`), storybook/preview servers (`preview-storybook`), and code
        // fixers (`//#fix`). Those need external services, hang, or mutate the
        // checkout, so they can never succeed in the sandbox and must not gate
        // automated pull-request creation.
        shouldValidate:
          !persistent && !longRunningTaskNames.has(name) && cache !== false
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
