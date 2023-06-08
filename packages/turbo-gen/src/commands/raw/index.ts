import { workspace, type TurboGeneratorCLIOptions } from "../workspace";
import { run, type CustomGeneratorCLIOptions } from "../run";
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
      // copy and empty needs to get massaged a bit when coming from rust
      let copy = false;
      let empty = incomingOptions.empty || true;

      // arg was passed with no value or as bool (explicitly)
      if (incomingOptions.copy === "" || incomingOptions.copy === true) {
        copy = true;
        empty = false;
        // arg was passed with a value
      } else if (incomingOptions.copy && incomingOptions.copy.length > 0) {
        copy = incomingOptions.copy;
        empty = false;
      }

      // update options values
      incomingOptions.copy = copy;
      incomingOptions.empty = empty;
      await workspace(incomingOptions as TurboGeneratorCLIOptions);
      break;
    case "run":
      const { generatorName, ...options } = incomingOptions;
      await run(generatorName, options as CustomGeneratorCLIOptions);
      break;
    default:
      console.error(
        `Received unknown command - "${command}" (must be one of "workspace" | "run")`
      );
      process.exit(1);
  }
}
