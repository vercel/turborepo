import { findRootSync } from "@manypkg/find-root";
import searchUp from "./searchUp";
import JSON5 from "json5";

function getTurboRoot(cwd?: string): string | null {
  const contentCheck = (content: string) => {
    const result = JSON5.parse(content);
    return !result.extends;
  };
  // Turborepo root can be determined by the presence of turbo.json
  let root = searchUp({
    target: "turbo.json",
    cwd: cwd || process.cwd(),
    contentCheck,
  });

  if (!root) {
    try {
      root = findRootSync(cwd || process.cwd());
      if (!root) {
        return null;
      }
    } catch (err) {
      return null;
    }
  }
  return root;
}

export default getTurboRoot;
