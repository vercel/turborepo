import path from "node:path";
import fs from "fs-extra";
import { dim } from "picocolors";
import { isFolderEmpty } from "./isFolderEmpty";

export function validateDirectory(directory: string): {
  valid: boolean;
  root: string;
  projectName: string;
  error?: string;
} {
  const root = path.resolve(directory);
  const projectName = path.basename(root);
  const exists = fs.existsSync(root);

  const stat = fs.lstatSync(root, { throwIfNoEntry: false });
  if (stat && !stat.isDirectory()) {
    return {
      valid: false,
      root,
      projectName,
      error: `${dim(
        projectName
      )} is not a directory - please try a different location`,
    };
  }

  if (exists) {
    const { isEmpty, conflicts } = isFolderEmpty(root);
    if (!isEmpty) {
      return {
        valid: false,
        root,
        projectName,
        error: `${dim(projectName)} (${root}) has ${
          conflicts.length
        } conflicting ${
          conflicts.length === 1 ? "file" : "files"
        } - please try a different location`,
      };
    }
  }

  return { valid: true, root, projectName };
}
