import { Flags } from "../types";
import path from "path";
import fs from "fs-extra";
import { error, ok, skip } from "../logger";

const DEFAULT_OUTPUTS = ["dist/**/*", "build/**/*"];

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

  // TODO: handle config in package.json turbo keys?

  const dir = files[0];
  const root = path.resolve(process.cwd(), dir);

  console.log(`Adding default \`outputs\` key into tasks if it doesn't exist`);

  const turboConfigPath = path.join(root, "turbo.json");
  if (!fs.existsSync(turboConfigPath)) {
    error(`No turbo.json found at ${root}. Is the path correct?`);
    process.exit(1);
  }

  const rootTurboJson: PipelineConfig = fs.readJsonSync(turboConfigPath);

  for (const [taskName, taskDef] of Object.entries(rootTurboJson.pipeline)) {
    if (!taskDef.outputs) {
      ok(`Updating outputs for ${taskName}`);
      taskDef.outputs = DEFAULT_OUTPUTS;
    } else if (Array.isArray(taskDef.outputs) && taskDef.outputs.length === 0) {
      ok(
        `Removing outputs: [] from ${taskName} as that is now the default behavior`
      );
      delete taskDef.outputs;
    } else {
      skip(`Skipping "${taskName}", it already has an outputs key defined`);
    }
  }

  if (!flags.dry) {
    fs.writeJsonSync(turboConfigPath, rootTurboJson, {
      spaces: 2,
    });
  } else {
    console.log(JSON.stringify(rootTurboJson, null, 2));
  }

  ok("Done");
}
