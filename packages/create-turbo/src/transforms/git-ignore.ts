import path from "path";
import fs from "fs-extra";
import { DEFAULT_IGNORE } from "../utils/git";
import { TransformInput, TransformResult } from "./types";

export async function transform(args: TransformInput): TransformResult {
  const { prompts } = args;
  const ignorePath = path.join(prompts.root, ".gitignore");
  try {
    if (!fs.existsSync(ignorePath)) {
      fs.writeFileSync(ignorePath, DEFAULT_IGNORE);
    } else {
      return { result: "not-applicable" };
    }
  } catch (err) {
    return { result: "error" };
  }

  return { result: "success" };
}
