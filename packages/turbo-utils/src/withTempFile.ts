import fs from "node:fs";
import os from "node:os";
import path from "node:path";

export function withTempFile<T>(task: (path: string) => T): T {
  const dir = fs.mkdtempSync(fs.realpathSync(os.tmpdir() + path.sep));
  const file = path.join(dir, "file");
  try {
    return task(file);
  } finally {
    fs.rmSync(dir, { recursive: true });
  }
}
