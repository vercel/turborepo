import path from "node:path";

import { defineTool } from "eve/tools";
import { z } from "zod";

import {
  getExamplePath,
  isJsonObject,
  pickJsonObject,
  readJsonFile
} from "../lib/repo.js";

interface TaskFinding {
  name: string;
  persistent: boolean;
  cache: boolean | null;
  scriptExists: boolean;
  shouldValidate: boolean;
}

const longRunningScriptNames = new Set(["dev", "start", "serve", "preview"]);

export default defineTool({
  description:
    "Inspect an example's turbo.json and package scripts to identify persistent tasks and non-persistent validation tasks that should pass after updates.",
  inputSchema: z.object({
    example: z
      .string()
      .min(1)
      .describe("Directory name under examples/, for example 'basic'.")
  }),
  async execute({ example }) {
    const examplePath = await getExamplePath(example);
    const packageJson = await readJsonFile(
      path.join(examplePath, "package.json")
    );
    const scripts = pickJsonObject(packageJson.scripts) ?? {};
    const turboJson = await readJsonFile(path.join(examplePath, "turbo.json"));
    const tasks = collectTasks(turboJson);
    const scriptNames = Object.keys(scripts).sort();
    const turboTasks = Object.entries(tasks).map(
      ([name, config]): TaskFinding => {
        const taskConfig = isJsonObject(config) ? config : {};
        const persistent = taskConfig.persistent === true;
        const cache =
          typeof taskConfig.cache === "boolean" ? taskConfig.cache : null;
        const scriptExists = typeof scripts[name] === "string";
        return {
          name,
          persistent,
          cache,
          scriptExists,
          shouldValidate:
            scriptExists && !persistent && !longRunningScriptNames.has(name)
        };
      }
    );

    const scriptOnlyValidationTasks = scriptNames.filter(
      (name) => !tasks[name] && !longRunningScriptNames.has(name)
    );

    return {
      example,
      turboTasks: turboTasks.sort((a, b) => a.name.localeCompare(b.name)),
      scriptOnlyValidationTasks,
      recommendedScriptsToRun: [
        ...turboTasks
          .filter((task) => task.shouldValidate)
          .map((task) => task.name),
        ...scriptOnlyValidationTasks
      ].sort()
    };
  }
});

function collectTasks(
  turboJson: Record<string, unknown>
): Record<string, unknown> {
  if (isJsonObject(turboJson.tasks)) {
    return turboJson.tasks;
  }
  if (isJsonObject(turboJson.pipeline)) {
    return turboJson.pipeline;
  }
  return {};
}
