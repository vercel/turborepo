import path from "node:path";
import type { Project } from "@turbo/workspaces";
import fs from "fs-extra";
import { TEMPLATES } from "../templates/embedded";
import { GeneratorError } from "./error";

export async function setupFromTemplate({
  project,
  template
}: {
  project: Project;
  template: "ts" | "js";
}) {
  const configDirectory = path.join(project.paths.root, "turbo", "generators");

  const templateKey = `simple-${template}` as keyof typeof TEMPLATES;
  const templateSet = TEMPLATES[templateKey];

  // required to ensure we don't overwrite any existing files at this location
  if (await fs.pathExists(configDirectory)) {
    throw new GeneratorError(
      `Generator config directory already exists at ${configDirectory}`,
      { type: "config_directory_already_exists" }
    );
  }

  // write each embedded template file to disk
  for (const file of templateSet.files) {
    const filePath = path.join(configDirectory, file.path);
    await fs.mkdirp(path.dirname(filePath));
    await fs.writeFile(filePath, file.content, "utf-8");
  }
}
