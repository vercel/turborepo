import path from "node:path/posix";

import { defineTool } from "eve/tools";
import { z } from "zod";

import { auditExampleTasks } from "../lib/example-tasks.js";
import {
  exampleChangeFingerprint,
  writeValidationState
} from "../lib/example-validation.js";
import {
  getExamplePath,
  packageManagerName,
  readJsonFile,
  resolveAutomatedExample,
  runCommandOrThrow
} from "../lib/repo.js";
import { buildTurboRunCommand } from "../lib/turbo-command.js";

export default defineTool({
  description:
    "Run all selected non-persistent Turbo tasks for one example in a single command, continuing after task failures and failing the tool if the command does not succeed.",
  inputSchema: z.object({
    example: z
      .string()
      .min(1)
      .describe("Directory name under examples/, for example 'basic'."),
    tasks: z
      .array(z.string().min(1))
      .min(1)
      .describe(
        "Every relevant non-persistent Turbo task to run together, for example ['build', 'lint', 'check-types']."
      ),
    timeoutSeconds: z.number().int().positive().max(1200).default(300)
  }),
  async execute({ example, tasks, timeoutSeconds }, ctx) {
    const sandbox = await ctx.getSandbox();
    const effectiveExample =
      (await resolveAutomatedExample(
        sandbox,
        ctx.session.auth.current,
        ctx.session.id,
        example
      )) ?? example;
    const examplePath = await getExamplePath(sandbox, effectiveExample);
    const packageJson = await readJsonFile(
      sandbox,
      path.join(examplePath, "package.json")
    );
    const turboJson = await readJsonFile(
      sandbox,
      path.join(examplePath, "turbo.json")
    );
    const expectedTasks = auditExampleTasks(
      packageJson,
      turboJson
    ).recommendedTurboTasksToRun;
    const manager = packageManagerName(packageJson.packageManager) ?? "pnpm";
    const attemptedTasks = [...new Set(tasks)];
    const fingerprint = await exampleChangeFingerprint(
      sandbox,
      effectiveExample
    );
    await writeValidationState(sandbox, ctx.session.id, {
      example: effectiveExample,
      fingerprint,
      status: "pending",
      tasks: attemptedTasks
    });

    try {
      const command = buildTurboRunCommand(manager, tasks);
      if (
        command.tasks.length !== expectedTasks.length ||
        command.tasks.some((task) => !expectedTasks.includes(task))
      ) {
        throw new Error(
          `Validate every audited non-persistent Turbo task in one call. Expected: ${expectedTasks.join(", ")}.`
        );
      }
      const result = await runCommandOrThrow(
        sandbox,
        command.command,
        command.args,
        examplePath,
        timeoutSeconds * 1_000
      );
      await writeValidationState(sandbox, ctx.session.id, {
        example: effectiveExample,
        fingerprint: await exampleChangeFingerprint(sandbox, effectiveExample),
        status: "success",
        tasks: command.tasks
      });
      return result;
    } catch (error) {
      await writeValidationState(sandbox, ctx.session.id, {
        example: effectiveExample,
        fingerprint,
        status: "failed",
        tasks: attemptedTasks
      });
      throw error;
    }
  }
});
