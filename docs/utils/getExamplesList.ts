import { readdirSync, lstatSync, readFileSync } from "node:fs";
import path from "node:path";

export const getExamplesList = () => {
  // Path to examples directory at the monorepo root.
  const examplesDirectory = path.join(
    __dirname,
    process.env.NODE_ENV === "production"
      ? "../../../../examples"
      : "../../../../../../../../examples"
  );
  const examples: {
    slug: string;
    name: string;
    description: string;
    template: string;
    featured?: true;
    boost?: true;
  }[] = [];
  const excludedExamples: string[] = [];
  readdirSync(examplesDirectory).forEach((file) => {
    if (lstatSync(path.join(examplesDirectory, file)).isDirectory()) {
      try {
        examples.push({
          slug: file,
          ...JSON.parse(
            readFileSync(
              path.join(examplesDirectory, file, "meta.json")
            ).toString()
          ),
        });
      } catch (err) {
        excludedExamples.push(file);
      }
    }
  });

  // throw an error if no examples are found
  if (examples.length === 0) {
    throw new Error(
      `No examples found in ${examplesDirectory}! Make sure you have updated the path if moving this file.`
    );
  }

  console.log(
    `Examples excluded due to missing meta.json: ${excludedExamples.join(", ")}`
  );
  return examples;
};
