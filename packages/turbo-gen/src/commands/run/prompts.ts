import { prompt, Separator } from "inquirer";
import { logger } from "@turbo/utils";
import type { Generator } from "../../utils/plop";

export async function customGenerators({
  generators,
  generator,
}: {
  generators: Array<Generator | Separator>;
  generator?: string;
}) {
  if (generator) {
    if (
      generators.find((g) => !(g instanceof Separator) && g.name === generator)
    ) {
      return {
        selectedGenerator: generator,
      };
    }

    logger.warn(`Generator "${generator}" not found`);
    logger.log();
  }

  const generatorAnswer = await prompt<{
    selectedGenerator: string;
  }>({
    type: "list",
    name: "selectedGenerator",
    message: `Select generator to run`,
    choices: generators.map((gen) => {
      if (gen instanceof Separator) {
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
  return prompt<{ answer: "ts" | "js" }>({
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
  return prompt<{ answer: boolean }>({
    type: "confirm",
    name: "answer",
    message,
  });
}
