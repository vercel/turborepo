import path from "node:path";
import fs from "node:fs";

export function directoryInfo({ directory }: { directory: string }) {
  const dir = path.isAbsolute(directory)
    ? directory
    : path.join(process.cwd(), directory);

  return { exists: fs.existsSync(dir), absolute: dir };
}
