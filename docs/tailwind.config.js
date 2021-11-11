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
        mono: [
          "Menlo",
          "Monaco",
          "Lucida Console",
          "Liberation Mono",
          "DejaVu Sans Mono",
          "Bitstream Vera Sans Mono",
          "Courier New",
          "monospace",
        ],
      },
      colors: {
        dark: "#111",
        gray: colors.trueGray,
        blue: colors.blue,
        orange: colors.orange,
        green: colors.green,
        red: colors.red,
        yellow: colors.yellow,
      },
    },
  },
  darkMode: "class",
};
