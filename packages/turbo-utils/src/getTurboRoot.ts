import { findRootSync } from "@manypkg/find-root";
import searchUp from "./searchUp";

function getTurboRoot(cwd?: string): string | null {
  // Turborepo root can be determined by the presence of turbo.json
  let root = searchUp({ target: "turbo.json", cwd: cwd || process.cwd() });

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
