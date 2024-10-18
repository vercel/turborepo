import { readFileSync } from "fs-extra";
import { parse as JSON5Parse } from "json5";

export function loadTurboJson<T>(filePath: string): T {
  const contents = readFileSync(filePath, "utf8");
  return JSON5Parse(contents);
}
