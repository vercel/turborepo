import path from "path";
import fs from "fs/promises";

const environmentPaths = (process.env.PATH || "")
  .replace(/["]+/g, "")
  .split(path.delimiter)
  .filter(Boolean);

const environmentExtensions = (process.env.PATHEXT || "").split(";");

/**
 * Determines whether or not the given executable is present on the system.
 *
 * Inspired by https://github.com/springernature/hasbin/blob/master/lib/hasbin.js#L55
 *
 * @param name
 */
export async function hasExecutable(
  executable: string,
  paths = environmentPaths,
  extensions = environmentExtensions
) {
  try {
    return await Promise.any(
      paths
        .flatMap((d) => extensions.map((ext) => path.join(d, executable + ext)))
        .map(isFilePresent)
    );
  } catch (err) {
    return false;
  }
}

async function isFilePresent(path: string) {
  const stats = await fs.stat(path);

  if (stats.isFile()) {
    return true;
  }

  throw new Error(`${path} is not a file`);
}
