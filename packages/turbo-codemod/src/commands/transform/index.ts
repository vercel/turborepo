import { bold, cyan, dim, gray } from "picocolors";
import { prompt } from "inquirer";
import { logger } from "@turbo/utils";
import { loadTransformers } from "../../utils/loadTransformers";
import { checkGitStatus } from "../../utils/checkGitStatus";
import { directoryInfo } from "../../utils/directoryInfo";
import { Runner } from "../../runner";
import type {
  TransformCommandOptions,
  TransformCommandArgument,
} from "./types";

export async function transform(
  transformName: TransformCommandArgument,
  directory: TransformCommandArgument,
  options: TransformCommandOptions
) {
  const transforms = loadTransformers();
  if (options.list) {
    logger.log(transforms.map((t) => `- ${cyan(t.name)}`).join("\n"));
    return process.exit(0);
  }

  // check git status
  if (!options.dryRun) {
    checkGitStatus({ directory, force: options.force });
  }

  const answers = await prompt<{
    directoryInput?: string;
    transformerInput?: string;
  }>([
    {
      type: "input",
      name: "directoryInput",
      message: "Where is the root of the repo where the transform should run?",
      when: !directory,
      default: ".",
      validate: (d: string) => {
        const { exists, absolute } = directoryInfo({ directory: d });
        if (exists) {
          return true;
        }
        return `Directory ${dim(`(${absolute})`)} does not exist`;
      },
      filter: (d: string) => d.trim(),
    },
    {
      type: "list",
      name: "transformerInput",
      message: "Which transform would you like to apply?",
      when: !transformName,
      pageSize: transforms.length,
      choices: transforms.map((t) => ({
        name: `${bold(t.name)} - ${gray(t.description)} ${gray(
          `(${t.introducedIn})`
        )}`,
        value: t.name,
      })),
    },
  ]);

  const {
    // eslint-disable-next-line @typescript-eslint/no-non-null-assertion -- we know it exists because of the prompt
    directoryInput: selectedDirectory = directory!,
    transformerInput: selectedTransformer = transformName,
  } = answers;
  const { exists, absolute: root } = directoryInfo({
    directory: selectedDirectory,
  });
  if (!exists) {
    logger.error(`Directory ${dim(`(${root})`)} does not exist`);
    return process.exit(1);
  }

  const transformKeys = transforms.map((t) => t.name);
  const transformData = transforms.find((t) => t.name === selectedTransformer);

  // validate transforms
  if (!transformData) {
    logger.error(
      `Invalid transform choice ${dim(`(${transformName})`)}, pick one of:`
    );
    logger.error(transformKeys.map((key) => `- ${key}`).join("\n"));
    return process.exit(1);
  }

  // run the transform
  const result = await transformData.transformer({
    root,
    options,
  });

  if (result.fatalError) {
    // Runner already logs this, so we can just exit
    return process.exit(1);
  }

  Runner.logResults(result);
}
