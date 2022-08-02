import fs from "fs";
import { getTurboRoot } from "turbo-utils";

import type { TurboConfig } from "../types";

function findTurboConfig(): TurboConfig | null {
  const turboRoot = getTurboRoot();
  if (turboRoot) {
    try {
      const raw = fs.readFileSync(`${turboRoot}/turbo.json`, "utf8");
      const turboJsonContent: TurboConfig = JSON.parse(raw);
      return turboJsonContent;
    } catch (e) {
      console.error(e);
      return null;
    }
  }

  return null;
}

export default findTurboConfig;
