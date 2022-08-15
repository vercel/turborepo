#!/usr/bin/env node
import shelljs from "shelljs";
import path from "path";
import fs from "fs-extra";
import faker from "faker";
import graphGenerator from "ngraph.generators";
import copy from "copy-template-dir";
import { fileURLToPath } from "url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
faker.seed(123);

const scope = `@${faker.hacker
  .noun()
  .toLowerCase()
  .replace(/\s/g, "-")
  .replace("1080p-", "rando")}`;

const type = process.argv[2];

// TODO: algo should be customizable along with the size
const packageGraph = graphGenerator.complete(5);

// Generate the package name & versions
packageGraph.forEachNode((node) => {
  node.data = {
    name: `${scope}/${faker.hacker.adjective()}-${faker.hacker.noun()}`
      .toLocaleLowerCase()
      .replace(/\s/g, "-"),
    version: faker.system.semver(),
  };
});

// Generate package dependencies
packageGraph.forEachNode((node) => {
  const links = packageGraph.getLinks(node.id);

  if (links) {
    for (const link of links) {
      if (link.fromId === node.id) {
        const depNode = packageGraph.getNode(link.toId);
        node.data.dependencies = node.data.dependencies || {};
        node.data.dependencies[depNode.data.name] = `^${depNode.data.version}`;
        node.data.implicitDependencies = node.data.implicitDependencies || [];
        node.data.implicitDependencies.push(
          depNode.data.name.replace(/^@[^/]+\//, "")
        );
      }
    }
  }
});

// Generate the monorepo
// 1. the root package.json
// 2. create packages/
// 3. create package directories
const root = path.join(__dirname, "../demo", type);

function generate(root, skipInstall) {
  fs.mkdirSync(root, { recursive: true });
  if (type !== "nx") {
    fs.writeFileSync(
      path.join(root, ".gitignore"),
      `node_modules
dist
.turbo
out
turbo
turbo-linux
.yalc`
    );
    if (fs.existsSync(root)) {
      try {
        fs.rmSync(root + "/packages", { recursive: true });
      } catch (error) {}
    }

    let deps =
      type !== "turbo"
        ? {
            devDependencies: {
              [type]: "*",
            },
          }
        : {};

    fs.writeFileSync(
      path.join(root, "package.json"),
      JSON.stringify(
        {
          name: "monorepo",
          version: "0.0.0",
          private: true,
          workspaces: ["packages/*"],
          ...deps,
          packageManager: "yarn@1.22.17",
        },
        null,
        2
      )
    );

    fs.writeFileSync(
      path.join(root, "tsconfig.json"),
      JSON.stringify(
        {
          compilerOptions: {
            composite: false,
            declaration: true,
            declarationMap: true,
            esModuleInterop: true,
            forceConsistentCasingInFileNames: true,
            inlineSourceMap: true,
            inlineSources: false,
            isolatedModules: true,
            moduleResolution: "node",
            noUnusedLocals: false,
            noUnusedParameters: false,
            preserveWatchOutput: true,
            skipLibCheck: true,
            strict: true,
            lib: ["es2020"],
            module: "commonjs",
            target: "es2020",
          },
        },
        null,
        2
      )
    );
  }

  if (type === "turbo") {
    fs.writeFileSync(
      path.join(root, "turbo.json"),
      JSON.stringify(
        {
          npmClient: "yarn",
          cacheStorageConfig: {
            provider: "local",
            cacheUrl: "https://1a77600385dd.ngrok.io",
          },
          pipeline: {
            build: {
              outputs: ["dist/**/*"],
              dependsOn: ["^build"],
            },
            test: {
              dependsOn: ["build"],
            },
            dev: {
              cache: false,
            },
          },
        },
        null,
        2
      )
    );
  }

  if (type === "lerna") {
    fs.writeFileSync(
      path.join(root, "lerna.json"),
      JSON.stringify(
        {
          packages: ["packages/*"],
          version: "0.0.0",
        },
        null,
        2
      )
    );
  }

  if (type === "lage") {
    fs.writeFileSync(
      path.join(root, "lage.config.js"),
      `
module.exports = {
  pipeline: {
    build: ['^build'],
    test: ['build'],
    lint: [],
  },
  npmClient: 'yarn',
  cacheOptions: {
    cacheStorageConfig: {
      provider: 'local',
    },
    outputGlob: ['dist/**'],
  },
};
    `
    );
  }

  if (type !== "nx") {
    fs.mkdirSync(path.join(root, "packages"));
  } else {
    shelljs.exec(
      `cd ${path.join(
        __dirname,
        "../demo"
      )} && yarn create nx-workspace nx --preset=empty --nx-cloud=false --packageManager=yarn --cli=nx --linter=eslint`
    );
    shelljs.exec(`cd ${root} && yarn add @nrwl/node`);
  }

  if (type !== "nx") {
    packageGraph.forEachNode((node) => {
      const packageRoot = path.join(
        root,
        "packages",
        node.data.name.replace(/^@[^/]+\//, "")
      );
      fs.mkdirSync(packageRoot, { recursive: true });
      copy(
        path.join(__dirname, "templates"),
        path.join(packageRoot),
        {
          name: node.data.name.replace(/^@[^/]+\//, ""),
        },
        () => {}
      );

      fs.writeFileSync(
        path.join(packageRoot, "package.json"),
        JSON.stringify(
          {
            name: node.data.name,
            version: node.data.version,
            dependencies: node.data.dependencies,
            files: ["dist/**"],
            main: "dist/index.js",
            types: "dist/index.d.ts",
            devDependencies: {
              typescript: "^4.6.3",
              jest: "^27.0.0",
              "ts-jest": "^27.0.0",
              "@types/jest": "^27.0.0",
            },
            scripts: {
              build: "tsc",
              dev: "tsc -w",
              test: "jest",
            },
          },
          null,
          2
        )
      );
    });
  }

  if (type === "nx") {
    packageGraph.forEachNode((node) => {
      shelljs.exec(
        `cd ${root} && yarn nx g @nrwl/node:library --buildable --publishable --name="${node.data.name.replace(
          /^@[^/]+\//,
          ""
        )}" --importPath="${node.data.name.replace(/^@[^/]+\//, "")}"`
      );
      // instead of dealing with actual code, just list as implicitDependencies
      const safeName = node.data.name.replace(/^@[^/]+\//, "");
      const workspace = fs.readJSONSync(path.join(root, "workspace.json"));
      workspace.projects[safeName] = {
        ...workspace.projects[safeName],
        implicitDependencies: node.data.implicitDependencies || [],
      };
      fs.writeFileSync(
        path.join(root, "nx.json"),
        JSON.stringify(workspace, null, 2)
      );
    });
  }
  if (!skipInstall) {
    shelljs.exec(`cd ${root} && yarn install`);
  }
  fs.ensureDirSync(path.join(root, ".git"));
  fs.writeFileSync(
    path.join(root, ".git", "config"),
    `
[user]
	name = GitHub Actions
	email = actions@users.noreply.github.com
`
  );
  shelljs.exec(
    `cd ${root} && git init -q && git add . && git commit -m "init"`
  );
}

generate(root);
if (type === "turbo") {
  generate(root + "-installed", true);
}
