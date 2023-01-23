import { TransformerOptions } from "../types";
import { Runner } from "../runner";
import Logger from "./logger";

export default function getTransformerHelpers({
  transformer,
  rootPath,
  options,
}: {
  transformer: string;
  rootPath: string;
  options: TransformerOptions;
}) {
  const utilArgs = {
    transformer,
    rootPath,
    ...options,
  };
  const log = new Logger(utilArgs);
  const runner = new Runner(utilArgs);

  return { log, runner };
}
