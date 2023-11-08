import { convertCase, logger } from "@turbo/utils";
import { workspace, type TurboGeneratorCLIOptions } from "../workspace";
import { run, type CustomGeneratorCLIOptions } from "../run";

interface CustomRawArgs extends CustomGeneratorCLIOptions {
  generatorName?: string;
}

type WorkspaceRawArgs = Omit<
  TurboGeneratorCLIOptions,
  "empty" | "showAllDependencies"
> & {
  // these have to be made optional
  empty?: boolean;
  showAllDependencies?: boolean;
};

// 🐪
function parseToCamel<T>(str: string): T {
  try {
    const parsed = JSON.parse(str) as Record<string, unknown>;
    const camelCased: Record<string, unknown> = {};
    for (const key in parsed) {
      const camelKey = convertCase(key, { to: "camel" });
      camelCased[camelKey] = parsed[key];
    }
    return camelCased as T;
  } catch (err) {
    logger.error("Error parsing arguments", err);
    process.exit(1);
  }
}

/**
 * Given a command and a JSON string of options, attempt to deserialize the JSON and run the command
 *
 * Used by the turbo Rust cli to handoff commands to the \@turbo/gen binary
 */
export async function raw(command: string, options: { json: string }) {
  if (command === "workspace") {
    const parsedArgs = parseToCamel<WorkspaceRawArgs>(options.json);
    parsedArgs.showAllDependencies = parsedArgs.showAllDependencies ?? false;

    // massage copy and empty
    let copy: string | boolean = false;
    let empty: boolean = parsedArgs.empty || true;

    // arg was passed with no value or as bool (explicitly)
    if (parsedArgs.copy === "" || parsedArgs.copy === true) {
      copy = true;
      empty = false;
      // arg was passed with a value
    } else if (parsedArgs.copy && parsedArgs.copy.length > 0) {
      copy = parsedArgs.copy;
      empty = false;
    }

    // update options values
    parsedArgs.copy = copy;
    parsedArgs.empty = empty;

    await workspace(parsedArgs as TurboGeneratorCLIOptions);
  } else if (command === "run") {
    const parsedArgs = parseToCamel<CustomRawArgs>(options.json);
    const { generatorName, ...rest } = parsedArgs;
    await run(generatorName, rest);
  } else {
    logger.error(
      `Received unknown command - "${command}" (must be one of "workspace" | "run")`
    );
    process.exit(1);
  }
}
