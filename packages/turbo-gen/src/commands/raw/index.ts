import { add, type TurboGeneratorOptions } from "../add";
import { generate, type CustomGeneratorOptions } from "../generate";
import { convertCase } from "@turbo/utils";

interface MinimalOptions {
  generatorName?: string;
  [arg: string]: any;
}

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
    case "add":
      await add(incomingOptions as TurboGeneratorOptions);
      break;
    case "generate":
      const { generatorName } = incomingOptions;
      await generate(generatorName, incomingOptions as CustomGeneratorOptions);
      break;
    default:
      console.error(
        `Received unknown command - "${command}" (must be one of "add" | "generate")`
      );
      process.exit(1);
  }
}
