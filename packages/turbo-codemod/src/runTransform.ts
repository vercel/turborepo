import path from "path";

import { Flags } from "./types";

export const transformerDirectory = path.join(__dirname, "transforms");

export interface RunTransformOptions {
  files: string[];
  flags: Flags;
  transformer: string;
}

export function runTransform({
  files,
  flags,
  transformer,
}: RunTransformOptions) {
  const transformerPath = path.join(transformerDirectory, `${transformer}.js`);
  return require(transformerPath).default(files, flags);
}
