import fs from "node:fs";
import path from "node:path";
import { z } from "zod";

const ExampleMetaSchema = z
  .object({
    slug: z.string(),
    name: z.string(),
    description: z.string(),
    template: z.string().optional(),
    maintainedByCoreTeam: z.boolean(),
  })
  .strict();

// Collect metadata from each example
const EXAMPLES = [];

// Get all directories in the examples folder
const examplesDir = path.join(process.cwd(), "../../examples");
const examples = fs
  .readdirSync(examplesDir, { withFileTypes: true })
  .filter(
    (dirent) =>
      dirent.isDirectory() &&
      !dirent.name.startsWith(".") &&
      dirent.name !== "node_modules"
  )
  .filter((dirent) => dirent.name !== "with-nextjs")
  .sort((a, b) => a.name.localeCompare(b.name))
  .map((dirent) => dirent.name);

for (const example of examples) {
  const metaPath = path.join(examplesDir, example, "meta.json");

  // Check if meta.json exists
  if (fs.existsSync(metaPath)) {
    try {
      const metaContent = fs.readFileSync(metaPath, "utf8");
      const metaJson = JSON.parse(metaContent) as z.infer<
        typeof ExampleMetaSchema
      >;
      EXAMPLES.push({ ...metaJson, slug: example });
    } catch (error) {
      // Ensure error is converted to string when creating new Error
      throw new Error(error instanceof Error ? error.message : String(error));
    }
  }
}

// Validate examples against schema
z.array(ExampleMetaSchema).parse(EXAMPLES);

// Write data to JSON file
const outputPath = path.join(process.cwd(), "./content/examples-data.json");
fs.writeFileSync(outputPath, JSON.stringify(EXAMPLES, null, 2), "utf8");

console.log(`Examples data written to ${outputPath}`);
