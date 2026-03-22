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
    const matchingGenerators = generators.filter(
      (g): g is Generator =>
        !(g instanceof Separator) &&
        (g.name === generator ||
          g.displayName === generator ||
          g.originalName === generator)
    );

    if (matchingGenerators.length === 1) {
      return {
        selectedGenerator: matchingGenerators[0].name
      };
    }

    if (matchingGenerators.length > 1) {
      logger.warn(
        `Generator "${generator}" is ambiguous. Use the package-qualified name in the prompt.`
      );
      logger.log();
    } else {
      logger.warn(`Generator "${generator}" not found`);
      logger.log();
    }
  }

  const selectedGenerator = await select({
    message: `Select generator to run`,
    choices: generators.map((gen) => {
      if (gen instanceof Separator) {
        return gen;
      }
      return {
        name: gen.description
          ? `  ${gen.displayName ?? gen.name}: ${gen.description}`
          : `  ${gen.displayName ?? gen.name}`,
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
