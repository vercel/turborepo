import path from "node:path";
import type { Project } from "@turbo/workspaces";
import fs from "fs-extra";
import { GeneratorError } from "./error";
import { TEMPLATES } from "../templates/embedded";

export async function setupFromTemplate({
  project,
  template
}: {
  project: Project;
  template: "ts" | "js";
}) {
  const configDirectory = path.join(project.paths.root, "turbo", "generators");

  const templateKey = `simple-${template}`;
  const embeddedTemplate = TEMPLATES[templateKey];
  if (!embeddedTemplate) {
    throw new GeneratorError(`Unknown template "${templateKey}"`, {
      type: "config_directory_already_exists"
    });
  }

  if (await fs.pathExists(configDirectory)) {
    throw new GeneratorError(
      `Generator config directory already exists at ${configDirectory}`,
      { type: "config_directory_already_exists" }
    );
  }

  for (const file of embeddedTemplate.files) {
    const filePath = path.join(configDirectory, file.relativePath);
    await fs.outputFile(filePath, file.content);
  }
}
