/**
 * Embedded template files for binary compilation.
 *
 * Templates are stored as string constants so they can be bundled into the
 * compiled binary without relying on filesystem paths at runtime.
 */

interface TemplateFile {
  /** Path relative to the template root (e.g. "config.ts") */
  path: string;
  content: string;
}

export interface TemplateSet {
  files: TemplateFile[];
}

export const TEMPLATES: Record<"simple-ts" | "simple-js", TemplateSet> = {
  "simple-ts": {
    files: [
      {
        path: "config.ts",
        content: `import { PlopTypes } from "@turbo/gen";

export default function generator(plop: PlopTypes.NodePlopAPI): void {
  plop.setGenerator("example", {
    description:
      "An example Turborepo generator - creates a new file at the root of the project",
    prompts: [
      {
        type: "input",
        name: "file",
        message: "What is the name of the new file to create?",
        validate: (input: string) => {
          if (input.includes(".")) {
            return "file name cannot include an extension";
          }
          if (input.includes(" ")) {
            return "file name cannot include spaces";
          }
          if (!input) {
            return "file name is required";
          }
          return true;
        }
      },
      {
        type: "list",
        name: "type",
        message: "What type of file should be created?",
        choices: [".md", ".txt"]
      },
      {
        type: "input",
        name: "title",
        message: "What should be the title of the new file?"
      }
    ],
    actions: [
      {
        type: "add",
        path: "{{ turbo.paths.root }}/{{ dashCase file }}{{ type }}",
        templateFile: "templates/turborepo-generators.hbs"
      }
    ]
  });
}
`
      },
      {
        path: "package.json",
        content: `{
  "type": "commonjs"
}
`
      },
      {
        path: "templates/turborepo-generators.hbs",
        content: `# {{ title }}

### Created with Turborepo Generators

Read the docs at [turborepo.dev](https://turborepo.dev/docs/guides/generating-code).
`
      }
    ]
  },
  "simple-js": {
    files: [
      {
        path: "config.js",
        content: `module.exports = function generator(plop) {
  plop.setGenerator("example", {
    description:
      "An example Turborepo generator - creates a new file at the root of the project",
    prompts: [
      {
        type: "input",
        name: "file",
        message: "What is the name of the new file to create?",
        validate: (input) => {
          if (input.includes(".")) {
            return "file name cannot include an extension";
          }
          if (input.includes(" ")) {
            return "file name cannot include spaces";
          }
          if (!input) {
            return "file name is required";
          }
          return true;
        }
      },
      {
        type: "list",
        name: "type",
        message: "What type of file should be created?",
        choices: [".md", ".txt"]
      },
      {
        type: "input",
        name: "title",
        message: "What should be the title of the new file?"
      }
    ],
    actions: [
      {
        type: "add",
        path: "{{ turbo.paths.root }}/{{ dashCase file }}{{ type }}",
        templateFile: "templates/turborepo-generators.hbs"
      }
    ]
  });
};
`
      },
      {
        path: "package.json",
        content: `{
  "type": "commonjs"
}
`
      },
      {
        path: "templates/turborepo-generators.hbs",
        content: `# {{ title }}

### Created with Turborepo Generators

Read the docs at [turborepo.dev](https://turborepo.dev/docs/guides/generating-code).
`
      }
    ]
  }
};
