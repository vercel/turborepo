import path from "node:path";
import fs from "fs-extra";

const HINTS = ["package.json", "turbo.json", ".git"];

export function looksLikeRepo({ directory }: { directory: string }): boolean {
  return HINTS.some((hint) => fs.existsSync(path.join(directory, hint)));
}
