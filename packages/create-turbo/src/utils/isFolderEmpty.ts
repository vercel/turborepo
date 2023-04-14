import fs from "fs-extra";

const VALID_FILES = [
  ".DS_Store",
  ".git",
  ".gitattributes",
  ".gitignore",
  ".gitlab-ci.yml",
  ".hg",
  ".hgcheck",
  ".hgignore",
  ".idea",
  ".npmignore",
  ".travis.yml",
  "LICENSE",
  "Thumbs.db",
  "docs",
  "mkdocs.yml",
  "npm-debug.log",
  "yarn-debug.log",
  "yarn-error.log",
  "yarnrc.yml",
  ".yarn",
];

export function isFolderEmpty(root: string): {
  isEmpty: boolean;
  conflicts: Array<string>;
} {
  const conflicts = fs
    .readdirSync(root)
    .filter((file) => !VALID_FILES.includes(file))
    // Support IntelliJ IDEA-based editors
    .filter((file) => !/\.iml$/.test(file));

  return { isEmpty: conflicts.length === 0, conflicts };
}
