import {
  defineConfig,
  js,
  reactHooksPlugin,
  reactPlugin,
  ts,
} from "@rslint/core";

export default defineConfig([
  {
    ignores: ["dist/**", "node_modules/**"],
  },
  js.configs.recommended,
  ts.configs.recommended,
  reactPlugin.configs.recommended,
  reactHooksPlugin.configs.recommended,
]);
