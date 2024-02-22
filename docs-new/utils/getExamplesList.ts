import { readdirSync, lstatSync, readFileSync } from "fs";
import path from "path";

export const getExamplesList = () => {
  // path to examples directory at the monorepo root.
  const examplesDirectory = path.join(__dirname, "../../examples");
  const examples: Array<{
    slug: string;
    name: string;
    description: string;
    template: string;
    featured?: true;
    boost?: true;
  }> = [];
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
