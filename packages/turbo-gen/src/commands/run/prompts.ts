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

export async function chooseGeneratorTemplate() {
  return inquirer.prompt<{ answer: "ts" | "js" }>({
    type: "list",
    name: "answer",
    message: "Should the generator config be created with TS or JS?",
    default: "ts",
    choices: [
      {
        name: "JavaScript",
        value: "js",
      },
      {
        name: "TypeScript",
        value: "ts",
      },
    ],
  });
}

export async function confirm({ message }: { message: string }) {
  return inquirer.prompt<{ answer: boolean }>({
    type: "confirm",
    name: "answer",
    message,
  });
}
