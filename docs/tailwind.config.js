const colors = require("tailwindcss/colors");

module.exports = {
  mode: "jit",
  purge: [
    "./components/**/*.js",
    "./components/**/*.tsx",
    "./nextra-theme-docs/**/*.js",
    "./nextra-theme-docs/**/*.tsx",
    "./nextra-theme-docs/**/*.css",
    "./pages/**/*.md",
    "./pages/**/*.mdx",
    "./pages/**/*.tsx",
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
        dark: "#08090D",
        gray: colors.blueGray,
        blue: colors.blue,
        orange: colors.orange,
        green: colors.green,
        red: colors.red,
        yellow: colors.yellow,
      },
      screens: {
        sm: "640px",
        md: "768px",
        lg: "1024px",
        betterhover: { raw: "(hover: hover)" },
      },
    },
  },
  darkMode: "class",
};
