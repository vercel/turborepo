import fs from "node:fs";
import path from "node:path";

export function searchUp({
  target,
  cwd,
  contentCheck,
}: {
  target: string;
  cwd: string;
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
