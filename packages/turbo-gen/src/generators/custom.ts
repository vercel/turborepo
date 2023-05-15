import chalk from "chalk";
import { logger } from "@turbo/utils";
import { getCustomGenerators, runCustomGenerator } from "../utils/plop";
import * as prompts from "../commands/generate/prompts";
import type { CustomGeneratorArguments } from "./types";
import { GeneratorError } from "../utils/error";

export async function generate({
  generator,
  project,
  opts,
}: CustomGeneratorArguments) {
  const generators = getCustomGenerators({ project, configPath: opts.config });
  if (!generators.length) {
    logger.error(`No custom generators found.`);
    console.log();
    return;
  }
  const { selectedGenerator } = await prompts.customGenerators({
    generators,
    generator,
  });

  try {
    await runCustomGenerator({
      project,
      generator: selectedGenerator,
      bypassArgs: opts.args,
      configPath: opts.config,
    });
  } catch (err) {
    // pass any GeneratorErrors through to root
    if (err instanceof GeneratorError) {
      throw err;
    }

    // capture any other errors and throw as GeneratorErrors
    let message = "Failed to run custom generator";
    if (err instanceof Error) {
      message = err.message;
    }

    throw new GeneratorError(message, {
      type: "plop_error_running_generator",
    });
  }

  console.log();
  console.log(chalk.bold(logger.turboGradient(">>> Success!")));
}
