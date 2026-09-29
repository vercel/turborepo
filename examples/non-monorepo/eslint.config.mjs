import { defineConfig, globalIgnores } from "eslint/config";
import nextVitals from "eslint-config-next/core-web-vitals";
import nextTs from "eslint-config-next/typescript";

// eslint-plugin-react (bundled by eslint-config-next) does not yet support
// ESLint 10. Drop its plugin and rules while keeping the react-hooks,
// jsx-a11y, import, and @next/next coverage from eslint-config-next.
const withoutReactPlugin = (configs) =>
  configs.map((config) => {
    if (!config.plugins?.react) {
      return config;
    }
    const plugins = { ...config.plugins };
    delete plugins.react;
    const rules = Object.fromEntries(
      Object.entries(config.rules ?? {}).filter(
        ([ruleName]) => !ruleName.startsWith("react/"),
      ),
    );
    return { ...config, plugins, rules };
  });

const eslintConfig = defineConfig([
  ...withoutReactPlugin(nextVitals),
  ...nextTs,
  // Override default ignores of eslint-config-next.
  globalIgnores([
    // Default ignores of eslint-config-next:
    ".next/**",
    "out/**",
    "build/**",
    "next-env.d.ts",
  ]),
]);

export default eslintConfig;
