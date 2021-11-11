const colors = require("tailwindcss/colors");

module.exports = {
  mode: "jit",
  purge: [
    "./components/**/*.js",
    "./nextra-theme-docs/**/*.js",
    "./nextra-theme-docs/**/*.css",
    "./pages/**/*.md",
    "./pages/**/*.mdx",
    "./theme.config.js",
    "./styles.css",
  ],
  theme: {
    extend: {
      fontFamily: {
        sans: [`"Inter"`, "sans-serif"],
      },
      colors: {
        dark: "#111",
        gray: colors.trueGray,
        blue: colors.blue,
      },
    },
  },
  darkMode: "class",
};
