import typescript from "@rollup/plugin-typescript";

export default [
  {
    input: "Button.tsx",
    output: {
      file: "dist/button.js",
    },
  },
  {
    input: "Header.tsx",
    output: {
      file: "dist/header.js",
    },
  },
].map((entry) => ({ ...entry, plugins: [typescript()] }));
