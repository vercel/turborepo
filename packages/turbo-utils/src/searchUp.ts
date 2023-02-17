import fs from "fs";
import path from "path";

function searchUp({
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
  while (!found && cwd !== root) {
    if (contentCheck) {
      try {
        const content = fs.readFileSync(path.join(cwd, target)).toString();
        if (contentCheck(content)) {
          found = true;
          break;
        }
      } catch {
        // keep looking
      }
    } else {
      if (fs.existsSync(path.join(cwd, target))) {
        found = true;
        break;
      }
    }

    cwd = path.dirname(cwd);
  }

  if (found) {
    return cwd;
  }

  return null;
}

export default searchUp;
