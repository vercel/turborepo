import path from "node:path";
import fs from "fs-extra";
import type { Transformer } from "../types";

// transforms/ is a sibling when built in in dist/
export const transformerDirectory =
  process.env.NODE_ENV === "test"
    ? path.join(__dirname, "../transforms")
    : path.join(__dirname, "./transforms");

export function loadTransformers(): Array<Transformer> {
  const transformerFiles = fs.readdirSync(transformerDirectory);
  return transformerFiles
    .map((transformerFilename) => {
      const transformerPath = path.join(
        transformerDirectory,
        transformerFilename
      );
      try {
        // eslint-disable-next-line @typescript-eslint/no-var-requires -- dynamic import
        const transform = require(transformerPath) as { default: Transformer };
        return transform.default;
      } catch (e) {
        // we ignore this error because it's likely that the file is not a transformer (README, etc)
        return undefined;
      }
    })
    .filter(Boolean) as Array<Transformer>;
}
