import typescript from "@rollup/plugin-typescript";

export default {
  input: "index.tsx",
  output: {
    file: "dist/index.js",
    format: "cjs",
  },
  plugins: [typescript()],
};
