import chalk from "chalk";
import { logger } from "@turbo/utils";
import { getCustomGenerators, runCustomGenerator } from "../utils/plop";
import * as prompts from "../commands/generate/prompts";
import type { CustomGeneratorArguments } from "./types";

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

  await runCustomGenerator({
    project,
    generator: selectedGenerator,
    bypassArgs: opts.args,
    configPath: opts.config,
  });
  console.log();
  console.log(chalk.bold(logger.turboGradient(">>> Success!")));
}
