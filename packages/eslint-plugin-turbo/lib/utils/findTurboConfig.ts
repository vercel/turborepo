import fs from "fs";
import { getTurboRoot } from "turbo-utils";
import type { Schema } from "turbo-types";
import JSON5 from "json5";

function findTurboConfig({ cwd }: { cwd?: string }): Schema | null {
  const turboRoot = getTurboRoot(cwd);
  if (turboRoot) {
    try {
      const raw = fs.readFileSync(`${turboRoot}/turbo.json`, "utf8");
      const turboJsonContent: Schema = JSON5.parse(raw);
      return turboJsonContent;
    } catch (e) {
      console.error(e);
      return null;
    }
  }

  return null;
}

export default findTurboConfig;
