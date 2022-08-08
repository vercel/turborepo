import fs from "fs";
import path from "path";

function searchUp({
  target,
  cwd,
}: {
  target: string;
  cwd: string;
}): string | null {
  const root = path.parse(cwd).root;

  let found = false;
  while (!found && cwd !== root) {
    if (fs.existsSync(path.join(cwd, target))) {
      found = true;
      break;
    }

    cwd = path.dirname(cwd);
  }

  if (found) {
    return cwd;
  }

  return null;
}

export default searchUp;
