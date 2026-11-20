import { defineConfig } from "tsup";

export default defineConfig({
  entry: ["src/button.tsx"],
  format: ["cjs", "esm"],
  external: ["react", "react/jsx-runtime"],
  target: "es2022",
});
