import { logger } from "@turbo/utils";
import { getCustomGenerators, runCustomGenerator } from "../utils/plop";
import * as prompts from "../commands/run/prompts";
import { GeneratorError } from "../utils/error";
import { setupFromTemplate } from "../utils/setupFromTemplate";
import type { CustomGeneratorArguments } from "./types";

export async function generate({
  generator,
  project,
  opts,
}: CustomGeneratorArguments) {
  let isOnboarding = false;
  let generators = getCustomGenerators({ project, configPath: opts.config });
  if (!generators.length) {
    logger.error(`No generators found.`);
    logger.log();

    const { answer } = await prompts.confirm({
      message: `Would you like to add a config with a sample custom generator to ${project.name}?`,
    });

    if (answer) {
      isOnboarding = true;
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

      // make it obvious that we're done creating a generator, and now we're running it
      logger.log();
      logger.info(`Generator config successfully created!`);
      logger.info(`Loading generator config...`);
      logger.log();

      // fetch generators again, and continue to selection prompt
      generators = getCustomGenerators({ project, configPath: opts.config });

      // something went wrong and we weren't able to find our new custom generator
      if (!generators.length) {
        logger.error(`Error loading generator`);
        return;
      }
    } else {
      logger.log();
      logger.dimmed(
        "Learn more about custom Turborepo generators - https://turbo.build/repo/docs/core-concepts/monorepos/code-generation#custom-generators"
      );
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
    let message = "Failed to run generator";
    if (err instanceof Error) {
      message = err.message;
    }

    throw new GeneratorError(message, {
      type: "plop_error_running_generator",
    });
  } finally {
    if (isOnboarding) {
      logger.log();
      logger.info(`Congrats! You just ran your first Turborepo generator`);
      logger.dimmed(
        "Learn more about custom Turborepo generators - https://turbo.build/repo/docs/core-concepts/monorepos/code-generation#custom-generators"
      );
    }
  }

  logger.log();
  logger.bold(logger.turboGradient(">>> Success!"));
}
