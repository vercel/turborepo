import {
  select,
  confirm as inquirerConfirm,
  Separator
} from "@inquirer/prompts";
import { logger } from "@turbo/utils";
import type { Generator } from "../../utils/plop";

export async function customGenerators({
  generators,
  generator
}: {
  generators: Array<Generator | Separator>;
  generator?: string;
}) {
  if (generator) {
    if (
      generators.some((g) => !(g instanceof Separator) && g.name === generator)
    ) {
      return {
        selectedGenerator: generator
      };
    }

    logger.warn(`Generator "${generator}" not found`);
    logger.log();
  }

  const selectedGenerator = await select({
    message: `Select generator to run`,
    choices: generators.map((gen) => {
      if (gen instanceof Separator) {
        return gen;
      }
      return {
        name: gen.description
          ? `  ${gen.name}: ${gen.description}`
          : `  ${gen.name}`,
        value: gen.name
      };
    })
  });

  return { selectedGenerator };
}

export async function chooseGeneratorTemplate() {
  const answer = await select<"ts" | "js">({
    message: "Should the generator config be created with TS or JS?",
    default: "ts",
    choices: [
      {
        name: "JavaScript",
        value: "js" as const
      },
      {
        name: "TypeScript",
        value: "ts" as const
      }
    ]
  });

  return { answer };
}

export async function confirm({ message }: { message: string }) {
  const answer = await inquirerConfirm({
    message
  });

  return { answer };
}
