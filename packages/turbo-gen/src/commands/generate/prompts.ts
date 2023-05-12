import inquirer from "inquirer";
import type { Generator } from "../../utils/plop";
import { logger } from "@turbo/utils";

export async function customGenerators({
  generators,
  generator,
}: {
  generators: Array<Generator | inquirer.Separator>;
  generator?: string;
}) {
  if (generator) {
    if (
      generators.find(
        (g) => !(g instanceof inquirer.Separator) && g.name === generator
      )
    ) {
      return {
        selectedGenerator: generator,
      };
    }

    logger.warn(`Generator "${generator}" not found`);
    console.log();
  }

  const generatorAnswer = await inquirer.prompt<{
    selectedGenerator: string;
  }>({
    type: "list",
    name: "selectedGenerator",
    message: `Select generator to run`,
    choices: generators.map((gen) => {
      if (gen instanceof inquirer.Separator) {
        return gen;
      }
      return {
        name: gen.description
          ? `  ${gen.name}: ${gen.description}`
          : `  ${gen.name}`,
        value: gen.name,
      };
    }),
  });

  return generatorAnswer;
}
