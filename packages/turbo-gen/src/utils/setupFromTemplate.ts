import path from "node:path";
import type { Project } from "@turbo/workspaces";
import { pathExists, copy } from "fs-extra";
import { GeneratorError } from "./error";

export async function setupFromTemplate({
  project,
  template,
}: {
  project: Project;
  template: "ts" | "js";
}) {
  const configDirectory = path.join(project.paths.root, "turbo", "generators");

  // TODO: could create some more complex starters in the future
  const toCopy = `simple-${template}`;

  // required to ensure we don't overwrite any existing files at this location
  if (await pathExists(configDirectory)) {
    throw new GeneratorError(
      `Generator config directory already exists at ${configDirectory}`,
      { type: "config_directory_already_exists" }
    );
  }

  // copy templates to project
  await copy(path.join(__dirname, "templates", toCopy), configDirectory, {
    recursive: true,
  });
}
