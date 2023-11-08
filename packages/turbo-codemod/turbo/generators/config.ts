import path from "path";
import fs from "fs-extra";
import type { PlopTypes } from "@turbo/gen";

export default function generator(plop: PlopTypes.NodePlopAPI): void {
  plop.setGenerator("transformer", {
    description: "Add a new transformer",
    prompts: [
      {
        type: "input",
        name: "name",
        message: 'key for the transform (example: "create-turbo-config")',
      },
      {
        type: "input",
        name: "description",
        message:
          'description for the transform (example: "Create the `turbo.json` file from an existing "turbo" key in `package.json`")',
      },
      {
        type: "input",
        name: "introducedIn",
        message:
          'the semantic version of turbo where this change was introduced (example: "1.1.0")',
      },
    ],
    actions: [
      {
        type: "add",
        path: "src/transforms/{{name}}.ts",
        templateFile: "templates/transformer.hbs",
      },
      {
        type: "add",
        path: "__tests__/{{name}}.test.ts",
        templateFile: "templates/transformer.test.hbs",
      },
      function createFixturesDirectory(answers: { name?: string }) {
        if (!answers.name) {
          return "no name provided, skipping fixture directory creation";
        }

        const directory = path.join(
          // resolves to the root of the current workspace
          plop.getDestBasePath(),
          "__tests__",
          "__fixtures__",
          answers.name
        );
        fs.mkdirSync(directory);

        return `created empty ${directory} directory for fixtures`;
      },
    ],
  });
}
