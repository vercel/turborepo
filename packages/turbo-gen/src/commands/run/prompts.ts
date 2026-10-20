import {
  select,
  confirm as inquirerConfirm,
  Separator
} from "@inquirer/prompts";
import { logger } from "@turbo/utils";
import {
  qualifiedName,
  parseQualifiedName,
  type Generator
} from "../../utils/plop";

function findGenerator(
  generators: Array<Generator | Separator>,
  name: string
): Generator | undefined {
  const parsed = parseQualifiedName(name);

  if (parsed) {
    return generators.find(
      (g): g is Generator =>
        !(g instanceof Separator) &&
        g.name === parsed.generator &&
        g.workspace === parsed.workspace
    );
  }

  const matches = generators.filter(
    (g): g is Generator => !(g instanceof Separator) && g.name === name
  );

  if (matches.length === 1) {
    return matches[0];
  }

  if (matches.length > 1) {
    logger.warn(
      `Multiple generators named "${name}" found. Use a qualified name to disambiguate:`
    );
    for (const m of matches) {
      logger.item(qualifiedName(m));
    }
    logger.log();
  }

  return undefined;
}

export async function customGenerators({
  generators,
  generator
}: {
  generators: Array<Generator | Separator>;
  generator?: string;
}): Promise<{ selectedGenerator: Generator }> {
  if (generator) {
    const match = findGenerator(generators, generator);
    if (match) {
      return { selectedGenerator: match };
    }

    logger.warn(`Generator "${generator}" not found`);
    logger.log();
  }

  const selectedGenerator = await select<Generator>({
    message: `Select generator to run`,
    choices: generators.map((gen) => {
      if (gen instanceof Separator) {
        return gen;
      }
      const qName = qualifiedName(gen);
      return {
        name: gen.description ? `  ${qName}: ${gen.description}` : `  ${qName}`,
        value: gen
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
