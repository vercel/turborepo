import chalk from "chalk";
import { logger } from "@turbo/utils";
import { getCustomGenerators, runCustomGenerator } from "../utils/plop";
import * as prompts from "../commands/run/prompts";
import type { CustomGeneratorArguments } from "./types";
import { GeneratorError } from "../utils/error";
import { setupFromTemplate } from "../utils/setupFromTemplate";

export async function generate({
  generator,
  project,
  opts,
}: CustomGeneratorArguments) {
  let generators = getCustomGenerators({ project, configPath: opts.config });
  if (!generators.length) {
    logger.error(`No custom generators found.`);
    console.log();

    const { answer } = await prompts.confirm({
      message: `Would you like to add generators to ${project.name}?`,
    });

    if (answer) {
      const { answer: template } = await prompts.chooseGeneratorTemplate();
      try {
        await setupFromTemplate({ project, template });
      } catch (err) {
        if (err instanceof GeneratorError) {
          throw err;
        }
        logger.error(`Failed to create generator config`);
        throw err;
      }

      // fetch generators again, and continue to selection prompt
      generators = getCustomGenerators({ project, configPath: opts.config });
    } else {
      return;
    }
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
