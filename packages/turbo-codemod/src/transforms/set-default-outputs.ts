import { Flags } from "../types";
import path from "path";
import fs from "fs-extra";
import { error, ok, skip } from "../logger";
import chalk from "chalk";

const DEFAULT_OUTPUTS = ["dist/**", "build/**"];

interface TaskDefinition {
  outputs: [];
}

interface PipelineConfig {
  [taskName: string]: TaskDefinition;
}

export default function addDefaultOutputs(files: string[], flags: Flags) {
  // We should only get a directory as input
  if (files.length !== 1) {
    return;
  }

  const dir = files[0];
  const root = path.resolve(process.cwd(), dir);

  // If `turbo` key is detected in package.json, require user to run the other codemod first.
  const packageJsonPath = path.join(root, "package.json");
  // package.json should always exist, but if it doesn't, it would be a silly place to blow up this codemod
  let packageJSON = {};

  try {
    packageJSON = fs.readJSONSync(packageJsonPath);
  } catch (e) {
    // readJSONSync probably failed because the file doesn't exist
  }

  if ("turbo" in packageJSON) {
    throw new Error(
      '"turbo" key detected in package.json. Run `npx @turbo/codemod create-turbo-config` first'
    );
  }

  console.log(`Adding default \`outputs\` key into tasks if it doesn't exist`);

  const turboConfigPath = path.join(root, "turbo.json");
  if (!fs.existsSync(turboConfigPath)) {
    error(`No turbo.json found at ${root}. Is the path correct?`);
    process.exit(1);
  }

  const rootTurboJson: PipelineConfig = fs.readJsonSync(turboConfigPath);

  let skippedCount = 0;
  let modifiedCount = 0;
  let unmodifiedCount = 0;
  for (const [taskName, taskDef] of Object.entries(rootTurboJson.pipeline)) {
    if (!taskDef.outputs) {
      ok(`Updating outputs for ${taskName}`);
      if (flags.dry) {
        skippedCount++;
      } else {
        taskDef.outputs = DEFAULT_OUTPUTS;
        modifiedCount++;
      }
    } else if (Array.isArray(taskDef.outputs) && taskDef.outputs.length === 0) {
      ok(
        `Removing outputs: [] from ${taskName} as that is now the default behavior`
      );
      if (flags.dry) {
        skippedCount++;
      } else {
        delete taskDef.outputs;
        modifiedCount++;
      }
    } else {
      unmodifiedCount++;
      skip(`Skipping "${taskName}", it already has an outputs key defined`);
    }
  }

  if (flags.dry) {
    console.log(JSON.stringify(rootTurboJson, null, 2));
  } else {
    fs.writeJsonSync(turboConfigPath, rootTurboJson, {
      spaces: 2,
    });
  }

  console.log("All done.");
  console.log("Results:");
  console.log(chalk.red(`0 errors`));
  console.log(chalk.yellow(`${skippedCount} skipped`));
  console.log(chalk.yellow(`${unmodifiedCount} unmodified`));
  console.log(chalk.green(`${modifiedCount} modified`));
}
