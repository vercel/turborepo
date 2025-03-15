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

type ExampleMeta = z.infer<typeof ExampleMetaSchema>;

// Collect metadata from each example
const EXAMPLES: ExampleMeta[] = [];

// Get all directories in the examples folder
const examplesDir = path.join(process.cwd() + "../../../examples");
const examples = fs
  .readdirSync(examplesDir, { withFileTypes: true })
  .filter(
    (dirent) =>
      dirent.isDirectory() &&
      !dirent.name.startsWith(".") &&
      dirent.name !== "node_modules"
  )
  .filter((dirent) => dirent.name !== "with-nextjs")
  // @ts-expect-error
  .sort((a, b) => a.name - b.name)
  .map((dirent) => dirent.name);

for (const example of examples) {
  const metaPath = path.join(examplesDir, example, "meta.json");

  // Check if meta.json exists
  if (fs.existsSync(metaPath)) {
    try {
      const metaContent = fs.readFileSync(metaPath, "utf8");
      const metaJson = JSON.parse(metaContent);
      EXAMPLES.push({ ...metaJson, slug: example });
    } catch (error) {
      // @ts-expect-error
      throw new Error(error);
    }
  }
}

// Validate examples against schema
const validatedExamples = z.array(ExampleMetaSchema).parse(EXAMPLES);

export function ExamplesTable({
  coreMaintained,
}: {
  coreMaintained?: boolean;
}): JSX.Element {
  return (
    <div className="overflow-auto max-w-full">
      <table>
        <thead>
          <tr>
            <th>Name</th>
            <th>Description</th>
          </tr>
        </thead>
        <tbody>
          {EXAMPLES.filter((meta) =>
            coreMaintained
              ? meta.maintainedByCoreTeam
              : !meta.maintainedByCoreTeam
          ).map((example) => {
            return (
              <tr key={example.slug}>
                <td>
                  <a
                    href={`https://github.com/vercel/turborepo/tree/main/examples/${example.slug}`}
                    rel="noopener noreferrer"
                    target="_blank"
                  >
                    {example.name}
                  </a>
                </td>
                <td className="sm:text-wrap">{example.description}</td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}
