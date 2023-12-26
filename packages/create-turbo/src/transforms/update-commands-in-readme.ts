import path from "node:path";
import fs from "node:fs/promises";
import { TransformError } from "./errors";
import type { TransformInput, TransformResult} from "./types";

const meta = {
  name: "update-commands-in-readme",
};

export async function transform(args: TransformInput): TransformResult {
  const { prompts, example } = args;

  const isOfficialStarter =
    !example.repo ||
    (example.repo.username === "vercel" && example.repo.name === "turbo");

  if (!isOfficialStarter) {
    return { result: "not-applicable", ...meta };
  }

  const selectedPackageManager = prompts.packageManager;
  const readmeFilePath = path.join(prompts.root, "examples", "basic", "README.md");
  try {
    // Read the content of the file
    let data = await fs.readFile(readmeFilePath, "utf8");

    // an array of all the possible replacement strings.
    const replacements = ['pnpm run', 'npm run', 'yarn run', 'bun run', 'pnpm', 'npm', 'yarn', 'bun'];
    const replacementRegex = new RegExp(`\\b(?:${replacements.join('|')})\\b`, 'g');

    // Replace all occurrences of regex with selectedPackageManager
    data = data.replace(replacementRegex, `${selectedPackageManager} run`);

    // Write the updated content back to the file
    await fs.writeFile(readmeFilePath, data, "utf8");

  } catch (err) {
    throw new TransformError("Unable to update README.md", {
      transform: meta.name,
      fatal: false,
    });
  }
  return { result: "success", ...meta };
}