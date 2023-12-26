import fs from "node:fs";
import path from "node:path";
import type { PlopTypes } from "@turbo/gen";
import { releasePostStats } from "./utils";
import * as helpers from "./helpers";

export default function generator(plop: PlopTypes.NodePlopAPI): void {
  // add helpers for use in templates
  helpers.init(plop);

  // create generators
  plop.setGenerator("blog - release post", {
    description: "Add a new release post to the turbo.build blog",
    prompts: [
      {
        type: "input",
        name: "version",
        message:
          'The full semantic version of the new release (example: "1.9.0")',
        validate: (input: string) => {
          if (!/^\d+\.\d+\.\d+$/.exec(input)) {
            return "Version must be in the form of major.minor.patch";
          }
          return true;
        },
      },
      {
        type: "input",
        name: "prevVersion",
        message:
          'The full semantic version of the previous release (example: "1.8.0")',
        validate: (input: string) => {
          if (!/^\d+\.\d+\.\d+$/.exec(input)) {
            return "Version must be in the form of major.minor.patch";
          }
          return true;
        },
      },
      {
        type: "checkbox",
        choices: [
          "gregsoltis",
          "nathanhammond",
          "tomknickman",
          "anthonyshew",
          "jaredpalmer",
          "mehulkar",
          "chrisolszewski",
          "nicholasyang",
          "alexanderlyon",
        ],
        name: "authors",
        pageSize: 20,
        message: "Select all authors for the release blog post.",
      },
      {
        type: "input",
        name: "tagline",
        message:
          'What is the tagline for the release (example: "focuses on improving observability for your task runs to better understand your caching behavior")',
      },
      {
        type: "input",
        name: "headlineTitle1",
        message: "What is the first headline feature?",
      },
      {
        type: "input",
        name: "headlineTitle2",
        message: "What is the second headline feature?",
      },
      {
        type: "input",
        name: "headlineTitle3",
        message: "What is the third headline feature?",
      },
    ],
    actions: [
      // extend answers with data fetched asynchronously
      releasePostStats,
      {
        type: "add",
        path: "pages/blog/turbo-{{dashCase version}}.mdx",
        templateFile: "templates/release-blog-post.hbs",
      },
      {
        type: "append",
        path: "pages/blog/_meta.json",
        pattern: /"\*":\s\{(?<group>.|\n)*?\},/gm,
        template:
          '  "turbo-{{dashCase version}}": "Turborepo {{ majorMinor version }}",',
      },
    ],
  });

  plop.setGenerator("blog - update release post stats", {
    description: "Update stats in a release post",
    prompts: [
      {
        type: "list",
        name: "post",
        pageSize: 20,
        message: "Which release post should the stats be updated?",
        choices: () => {
          return (
            fs
              // getDestBasePath resolves to the root of the current workspace
              .readdirSync(path.join(plop.getDestBasePath(), "pages/blog"))
              .filter((f) => f.startsWith("turbo-"))
              .map((f) => ({
                name: f
                  .replace("turbo-", "")
                  .replace(".mdx", "")
                  .replace(/-/g, "."),
                value: f,
              }))
          );
        },
      },
    ],
    actions: [
      // extend answers with data fetched asynchronously
      releasePostStats,
      // update github stars
      {
        type: "modify",
        path: "pages/blog/{{ post }}",
        pattern: /^-\s\[.*?\sGitHub\sStars\].*$/gm,
        template:
          "- [{{ turboStars }}+ GitHub Stars](https://github.com/vercel/turbo)",
      },
      // update weekly npm downloads
      {
        type: "modify",
        path: "pages/blog/{{ post }}",
        pattern: /^-\s\[.*?\sweekly\sNPM\sdownloads\].*$/gm,
        template:
          "- [{{ turboDownloads }}+ weekly NPM downloads](https://www.npmjs.com/package/turbo)",
      },
      // update years saved
      {
        type: "modify",
        path: "pages/blog/{{ post }}",
        pattern: /^-\s.*?years of compute time saved.*$/gm,
        template:
          "- {{ turboYearsSaved }} years of compute time saved through [Remote Caching on Vercel](https://vercel.com/docs/concepts/monorepos/remote-caching)",
      },
    ],
  });
}
