import fs from "node:fs";
import path from "node:path";

/**
 * recursively search up the file tree looking for a `target` file, starting with the provided `cwd`
 *
 * If found, return the directory containing the file. If not found, return null.
 */
export function searchUp({
  target,
  cwd,
  contentCheck,
}: {
  /** The name of the file we're looking for */
  target: string;

  /** The directory to start the search */
  cwd: string;

  /** a predicate for examining the content of any found file */
  contentCheck?: (content: string) => boolean;
}): string | null {
  const root = path.parse(cwd).root;

  let found = false;
  let lastCwd = cwd;
  while (!found && lastCwd !== root) {
    if (contentCheck) {
      try {
        const content = fs.readFileSync(path.join(lastCwd, target)).toString();
        if (contentCheck(content)) {
          found = true;
          break;
        }
      } catch {
        // keep looking
      }
    } else if (fs.existsSync(path.join(lastCwd, target))) {
      found = true;
      break;
    }

    lastCwd = path.dirname(lastCwd);
  }

  if (found) {
    return lastCwd;
  }

  return null;
}
