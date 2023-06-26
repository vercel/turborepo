import path from "path";
import fs from "fs-extra";
import { DEFAULT_IGNORE } from "../utils/git";
import { TransformInput, TransformResult } from "./types";
import { TransformError } from "./errors";

const meta = {
  name: "git-ignore",
};

export async function transform(args: TransformInput): TransformResult {
  const { prompts } = args;
  const ignorePath = path.join(prompts.root, ".gitignore");
  try {
    if (!fs.existsSync(ignorePath)) {
      fs.writeFileSync(ignorePath, DEFAULT_IGNORE);
    } else {
      return { result: "not-applicable", ...meta };
    }
  } catch (err) {
    // existsSync cannot throw, so we don't need to narrow here and can
    // assume this came from writeFileSync
    throw new TransformError("Unable to write .gitignore", {
      transform: meta.name,
      fatal: false,
    });
  }

  return { result: "success", ...meta };
}
