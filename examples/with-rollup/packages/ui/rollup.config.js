import swc from "@rollup/plugin-swc";

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
].map((entry) => ({
  ...entry,
  external: ["react/jsx-runtime"],
  plugins: [
    swc({
      swc: {
        jsc: {
          transform: {
            react: {
              runtime: "automatic",
            },
          },
        },
      },
    }),
  ],
}));
