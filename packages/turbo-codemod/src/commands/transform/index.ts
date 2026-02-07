import picocolors from "picocolors";
import { input, select } from "@inquirer/prompts";
import { logger } from "@turbo/utils";
import { loadTransformers } from "../../utils/loadTransformers";
import { checkGitStatus } from "../../utils/checkGitStatus";
import { directoryInfo } from "../../utils/directoryInfo";
import { Runner } from "../../runner";
import type {
  TransformCommandOptions,
  TransformCommandArgument
} from "./types";

export async function transform(
  transformName: TransformCommandArgument,
  directory: TransformCommandArgument,
  options: TransformCommandOptions
) {
  const transforms = loadTransformers();
  if (options.list) {
    logger.log(
      transforms.map((t) => `- ${picocolors.cyan(t.name)}`).join("\n")
    );
    return process.exit(0);
  }

  // check git status
  if (!options.dryRun) {
    checkGitStatus({ directory, force: options.force });
  }

  let selectedDirectory = directory;
  if (!selectedDirectory) {
    selectedDirectory = await input({
      message: "Where is the root of the repo where the transform should run?",
      default: ".",
      validate: (d: string) => {
        const { exists, absolute } = directoryInfo({ directory: d });
        if (exists) {
          return true;
        }
        return `Directory ${picocolors.dim(`(${absolute})`)} does not exist`;
      },
      transformer: (d: string) => d.trim()
    });
    selectedDirectory = selectedDirectory.trim();
  }

  let selectedTransformer = transformName;
  if (!selectedTransformer) {
    selectedTransformer = await select({
      message: "Which transform would you like to apply?",
      choices: transforms.map((t) => ({
        name: `${picocolors.bold(t.name)} - ${picocolors.gray(
          t.description
        )} ${picocolors.gray(`(${t.introducedIn})`)}`,
        value: t.name
      }))
    });
  }

  const { exists, absolute: root } = directoryInfo({
    directory: selectedDirectory
  });
  if (!exists) {
    logger.error(`Directory ${picocolors.dim(`(${root})`)} does not exist`);
    return process.exit(1);
  }

  const transformKeys = transforms.map((t) => t.name);
  const transformData = transforms.find((t) => t.name === selectedTransformer);

  // validate transforms
  if (!transformData) {
    logger.error(
      `Invalid transform choice ${picocolors.dim(
        `(${transformName})`
      )}, pick one of:`
    );
    logger.error(transformKeys.map((key) => `- ${key}`).join("\n"));
    return process.exit(1);
  }

  // run the transform
  const result = await transformData.transformer({
    root,
    options
  });

  if (result.fatalError) {
    // Runner already logs this, so we can just exit
    return process.exit(1);
  }

  Runner.logResults(result);
}
