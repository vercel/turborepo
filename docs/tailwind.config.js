const { createPreset } = require("fumadocs-ui/tailwind-plugin");

/** @type {import('tailwindcss').Config} */
module.exports = {
  content: [
    "./components/**/*.{ts,tsx}",
    "./app/**/*.{ts,tsx}",
    "./content/**/*.{md,mdx}",
    "./mdx-components.{ts,tsx}",
    "./node_modules/fumadocs-ui/dist/**/*.js",
  ],
  theme: {
    extend: {
      typography: {
        DEFAULT: {
          css: {
            a: {
              textDecoration: "none",
              color: "#008AEA",
              "&:hover": {
                color: "#1D4ED8",
                textDecoration: "underline",
              },
            },
            h1: {
              textAlign: "center",
            },
          },
        },
      },
    },
  },
  plugins: [require("@tailwindcss/typography")],
  presets: [createPreset()],
};
