import classNames from "classnames";
import { useCallback, useState } from "react";
import merge from "deepmerge";

// stand-in for the `satisfies` keyword in 4.9
const satisfy =
  <T,>() =>
  <T2 extends T>(t2: T2) =>
    t2;

interface TurboPipeline {
  [task: string]: {
    dependsOn?: string[];
    outputs?: string[];
    cache?: false;
  };
}

interface Tool {
  label: string;
  turboPipeline?: TurboPipeline;
  appPackageJsonScripts?: Record<string, string>;
  pkgPackageJsonScripts?: Record<string, string>;
  rootPackageJsonScripts?: Record<string, string>;
}

const tools = satisfy<Record<string, Tool>>()({
  prettier: {
    label: "Prettier",
    rootPackageJsonScripts: {
      format: 'prettier --write "**/*.{ts,tsx,md}"',
    },
  },
  eslint: {
    label: "ESLint",
    turboPipeline: {
      lint: {
        outputs: [],
      },
    },
    appPackageJsonScripts: {
      lint: "eslint",
    },
  },
  typescriptAsLinter: {
    label: "TypeScript (as linter)",
    turboPipeline: {
      lint: {
        outputs: [],
      },
    },
    appPackageJsonScripts: {
      lint: "tsc",
    },
  },
  tsup: {
    label: "Tsup",
    turboPipeline: {
      build: {
        outputs: ["dist/**"],
      },
      dev: {
        cache: false,
      },
    },
    pkgPackageJsonScripts: {
      dev: "npm run build --watch",
      build: "tsup src/index.ts",
    },
  },
  nextjs: {
    label: "Next.js",
    turboPipeline: {
      build: {
        outputs: [".next/**"],
      },
      start: {
        // build must always be run before start
        dependsOn: ["build"],
        cache: false,
      },
      lint: {
        outputs: [],
      },
      dev: {
        cache: false,
      },
    },
    appPackageJsonScripts: {
      dev: "next",
      build: "next build",
      lint: "next lint",
      start: "next start",
    },
  },
  vitest: {
    label: "Vitest",
    turboPipeline: {
      test: {
        outputs: ["coverage"],
      },
      "test:watch": {
        cache: false,
      },
    },
    appPackageJsonScripts: {
      test: "vitest run --coverage",
      "test:watch": "vitest",
    },
  },
  jest: {
    label: "Jest",
    turboPipeline: {
      test: {
        outputs: ["coverage"],
      },
      "test:watch": {
        cache: false,
      },
    },
    appPackageJsonScripts: {
      test: "jest --coverage",
      "test:watch": "jest --watch",
    },
  },
  remix: {
    label: "Remix",
    turboPipeline: {
      build: {
        // By default, remix build outputs to build/**
        outputs: ["build/**"],
      },
      dev: {
        cache: false,
      },
    },
    appPackageJsonScripts: {
      dev: "remix dev",
      build: "remix build",
    },
  },
  cypress: {
    label: "Cypress",
    turboPipeline: {
      "e2e:test": {
        // Cache screenshots and videos
        outputs: ["cypress/screenshots/**", "cypress/videos/**"],
      },
      "e2e:test:watch": {
        // Acts as a kind of dev script, so should
        // never be cached
        cache: false,
      },
    },
    appPackageJsonScripts: {
      "e2e:test":
        'start-server-and-test "npm run dev" http://localhost:3000 "cypress run"',
      "e2e:test:watch": "cypress open",
    },
  },
  vite: {
    label: "Vite",
    turboPipeline: {
      build: {
        // By default, vite build outputs to dist/**
        outputs: ["dist/**"],
      },
      preview: {
        // build must always be run before preview
        dependsOn: ["build"],
        cache: false,
      },
      dev: {
        cache: false,
      },
    },
    appPackageJsonScripts: {
      dev: "vite",
      build: "vite build",
      preview: "vite preview",
    },
  },
  storybook: {
    label: "Storybook",
    turboPipeline: {
      "storybook:build": {
        outputs: ["storybook-static/**"],
      },
      "storybook:dev": {
        cache: false,
      },
    },
    appPackageJsonScripts: {
      "storybook:build": "build-storybook",
      "storybook:dev": "start-storybook",
    },
  },
});

const defaultSelected: ToolId[] = ["nextjs", "eslint", "typescriptAsLinter"];

const toolLabels = Object.entries(tools).map(([id, { label }]) => ({
  id: id as ToolId,
  label,
}));

type ToolId = keyof typeof tools;

export const MonorepoBuilder = () => {
  const [selectedTools, setSelectedTools] = useState<ToolId[]>(defaultSelected);

  const toggleTool = useCallback((tool: ToolId) => {
    setSelectedTools((tools) => {
      if (tools.includes(tool)) {
        return tools.filter((t) => t !== tool);
      } else {
        return [...tools, tool];
      }
    });
  }, []);

  return (
    <div>
      <h1 className="mb-6">Monorepo Builder</h1>
      <div className="grid grid-cols-6 gap-4">
        {toolLabels.map(({ id, label }) => {
          const isSelected = selectedTools.includes(id);
          return (
            <button
              onClick={() => toggleTool(id)}
              className={classNames(
                "px-4 py-2 text-sm text-gray-200 bg-gray-800 rounded",
                isSelected && `bg-blue-900`
              )}
            >
              {label}
            </button>
          );
        })}
      </div>
      {resolveFiles(selectedTools).map((file) => (
        <>
          <h3>
            <code>{file.path}</code>
          </h3>
          <pre key={file.path}>{JSON.stringify(file.content, null, 2)}</pre>
        </>
      ))}
    </div>
  );
};

interface File {
  path: string;
  content: {};
}

const resolveFiles = (toolIds: ToolId[]): File[] => {
  const ciTasksSet = new Set<string>();

  const turboConfig = {
    $schema: "https://turborepo.org/schema.json",
    pipeline: {},
  };

  const appPackageJson = {
    scripts: {},
  };

  const rootPackageJson = {
    scripts: {},
  };

  toolIds.forEach((toolId) => {
    const tool: Tool = tools[toolId];
    if (tool.turboPipeline) {
      turboConfig.pipeline = merge(
        turboConfig.pipeline,
        tool.turboPipeline as any
      );

      Object.entries(tool.turboPipeline).forEach(([name, config]) => {
        if (typeof config.cache === "undefined") {
          ciTasksSet.add(name);
        }
      });
    }

    Object.entries(tool.appPackageJsonScripts || {}).forEach(
      ([name, script]) => {
        if (!appPackageJson.scripts[name]) {
          appPackageJson.scripts[name] = script;
        } else {
          appPackageJson.scripts[name] += ` && ${script}`;
        }
      }
    );

    Object.entries(tool.rootPackageJsonScripts || {}).forEach(
      ([name, script]) => {
        if (!rootPackageJson.scripts[name]) {
          rootPackageJson.scripts[name] = script;
        } else {
          rootPackageJson.scripts[name] += ` && ${script}`;
        }
      }
    );
  });

  if (ciTasksSet.size) {
    rootPackageJson.scripts["ci"] = `turbo run ${Array.from(ciTasksSet).join(
      " "
    )}`;
  }

  return [
    {
      path: "turbo.json",
      content: turboConfig,
    },
    {
      path: "package.json",
      content: rootPackageJson,
    },
    {
      path: "apps/web/package.json",
      content: appPackageJson,
    },
  ];
};
