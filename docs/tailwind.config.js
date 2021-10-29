module.exports = {
  mode: "jit",
  purge: [
    "./components/**/*.ts",
    "./components/**/*.js",
    "./components/**/*.tsx",
    "./pages/**/*.md",
    "./pages/**/*.mdx",
    "./theme.config.js",
    "./styles.css",
  ],
  darkMode: "class",
  plugins: [
    require("@tailwindcss/typography"),
    require("@tailwindcss/forms"),
    // ...
  ],
};
