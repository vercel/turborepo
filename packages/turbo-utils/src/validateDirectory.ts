import path from "node:path";
import fs from "fs-extra";
import picocolors from "picocolors";
import { isFolderEmpty } from "./isFolderEmpty";

export function validateDirectory(directory: string): {
  valid: boolean;
  root: string;
  projectName: string;
  error?: string;
} {
  // Basic sanity checks on the provided directory string to prevent
  // unsafe paths from flowing into tooling that invokes git or other CLIs.
  if (
    !directory ||
    typeof directory !== "string" ||
    directory.trim() === "" ||
    directory.includes("\0")
  ) {
    const safeDirectory = typeof directory === "string" ? directory : "";
    return {
      valid: false,
      root: "",
      projectName: "",
      error: `${picocolors.dim(
        safeDirectory || "<empty>"
      )} is not a valid directory name - please try a different location`
    };
  }

  const root = path.resolve(directory);
  const projectName = path.basename(root);

  // Prevent resolved paths that could be misinterpreted as command-line options
  // when passed to tools like git, and ensure the project name is well-formed.
  const unsafeRoot = !root || root.startsWith("-") || root.includes("\0");
  const invalidProjectName =
    !projectName || !/^[a-zA-Z0-9._-]+$/.test(projectName);

  if (unsafeRoot || invalidProjectName) {
    return {
      valid: false,
      root,
      projectName,
      error: `${picocolors.dim(
        projectName || root || "<unknown>"
      )} is not a valid directory - please try a different location`
    };
  }

  const exists = fs.existsSync(root);

  const stat = fs.lstatSync(root, { throwIfNoEntry: false });
  if (stat && !stat.isDirectory()) {
    return {
      valid: false,
      root,
      projectName,
      error: `${picocolors.dim(
        projectName
      )} is not a directory - please try a different location`
    };
  }

  if (exists) {
    const { isEmpty, conflicts } = isFolderEmpty(root);
    if (!isEmpty) {
      return {
        valid: false,
        root,
        projectName,
        error: `${picocolors.dim(projectName)} (${root}) has ${
          conflicts.length
        } conflicting ${
          conflicts.length === 1 ? "file" : "files"
        } - please try a different location`
      };
    }
  }

  return { valid: true, root, projectName };
}
