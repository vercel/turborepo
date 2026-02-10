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
export function parseToCamel<T>(str: string): T {
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

export function parseWorkspaceArgs(json: string): TurboGeneratorCLIOptions {
  const parsedArgs = parseToCamel<WorkspaceRawArgs>(json);
  parsedArgs.showAllDependencies = parsedArgs.showAllDependencies ?? false;

  let copy: string | boolean = false;
  let empty: boolean = parsedArgs.empty || true;

  if (parsedArgs.copy === "" || parsedArgs.copy === true) {
    copy = true;
    empty = false;
  } else if (parsedArgs.copy && parsedArgs.copy.length > 0) {
    copy = parsedArgs.copy;
    empty = false;
  }

  parsedArgs.copy = copy;
  parsedArgs.empty = empty;

  return parsedArgs as TurboGeneratorCLIOptions;
}

export function parseRunArgs(json: string): {
  generatorName: string | undefined;
  rest: CustomGeneratorCLIOptions;
} {
  const parsedArgs = parseToCamel<CustomRawArgs>(json);
  const { generatorName, ...rest } = parsedArgs;
  return { generatorName, rest };
}

/**
 * Given a command and a JSON string of options, attempt to deserialize the JSON and run the command
 *
 * Used by the turbo Rust cli to handoff commands to the \@turbo/gen binary
 */
export async function raw(command: string, options: { json: string }) {
  if (command === "workspace") {
    await workspace(parseWorkspaceArgs(options.json));
  } else if (command === "run") {
    const { generatorName, rest } = parseRunArgs(options.json);
    await run(generatorName, rest);
  } else {
    logger.error(
      `Received unknown command - "${command}" (must be one of "workspace" | "run")`
    );
    process.exit(1);
  }
}
