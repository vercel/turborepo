import { workspace, type TurboGeneratorOptions } from "../workspace";
import { run, type CustomGeneratorOptions } from "../run";
import { convertCase } from "@turbo/utils";

interface MinimalOptions {
  generatorName?: string;
  [arg: string]: any;
}

/**
 * Given a command and a JSON string of options, attempt to deserialize the JSON and run the command
 *
 * Used by the turbo Rust cli to handoff commands to the @turbo/gen binary
 */
export async function raw(command: string, options: { json: string }) {
  let incomingOptions: MinimalOptions = {};
  try {
    const parsed = JSON.parse(options.json || "{}");
    // convert keys in parsed to camelCase and add to incomingOptions (if these are coming from rust they're likely kebab)
    for (const key in parsed) {
      incomingOptions[convertCase(key, { to: "camel" })] = parsed[key];
    }
  } catch (err) {
    console.error("Error parsing arguments", err);
    process.exit(1);
  }

  switch (command) {
    case "workspace":
      await workspace(incomingOptions as TurboGeneratorOptions);
      break;
    case "run":
      const { generatorName, ...options } = incomingOptions;
      await run(generatorName, options as CustomGeneratorOptions);
      break;
    default:
      console.error(
        `Received unknown command - "${command}" (must be one of "add" | "generate")`
      );
      process.exit(1);
  }
}
