import path from "path";
import fs from "fs";

export default function directoryInfo({ directory }: { directory: string }) {
  const dir = path.isAbsolute(directory)
    ? directory
    : path.join(process.cwd(), directory);

  return { exists: fs.existsSync(dir), absolute: dir };
}
