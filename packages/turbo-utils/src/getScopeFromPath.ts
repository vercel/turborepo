import fs from "fs";
import path from "path";
import type { Scope } from "./types";

function getScopeFromPath({ cwd }: { cwd: string }): Scope {
  const packageJsonPath = path.join(cwd, "package.json");
  try {
    const raw = fs.readFileSync(packageJsonPath, "utf8");
    const packageJsonContent: Record<string, string> & { name: string } =
      JSON.parse(raw);

    return {
      scope: packageJsonContent.name,
      context: { path: packageJsonPath },
    };
  } catch (e) {
    return { scope: null, context: {} };
  }
}

export default getScopeFromPath;
