import path from "path";
import fs from "fs-extra";
import type { Transformer } from "../types";

// transforms/ is a sibling when built in in dist/
export const transformerDirectory =
  process.env.NODE_ENV === "test"
    ? path.join(__dirname, "../transforms")
    : path.join(__dirname, "./transforms");

export default function loadTransformers(): Array<Transformer> {
  const transformerFiles = fs.readdirSync(transformerDirectory);
  return transformerFiles
    .map((transformerFilename) => {
      const transformerPath = path.join(
        transformerDirectory,
        transformerFilename
      );
      try {
        return require(transformerPath).default;
      } catch (e) {
        // we ignore this error because it's likely that the file is not a transformer (README, etc)
        return undefined;
      }
    })
    .filter(Boolean);
}
