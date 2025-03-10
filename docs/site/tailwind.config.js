const { createPreset } = require("fumadocs-ui/tailwind-plugin");

/** @type {import('tailwindcss').Config} */
module.exports = {
  content: [
    "./components/**/*.{ts,tsx}",
    "./app/**/*.{ts,tsx}",
    "./content/**/*.{md,mdx}",
    "./mdx-components.{ts,tsx}",
    "./node_modules/fumadocs-ui/dist/**/*.js",
    "./node_modules/fumadocs-openapi/dist/**/*.js",
  ],
  theme: {
    extend: {
      fontFamily: {
        sans: ["var(--font-geist-sans)"],
        mono: ["var(--font-geist-mono)"],
      },
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
              fontWeight: 700,
              letterSpacing: "-0.02em",
            },
            h2: {
              fontSize: "1.75em",
              fontWeight: 600,
            },
            h3: {
              fontWeight: 500,
            },
            h4: {
              fontWeight: 500,
            },
          },
        },
      },
    },
  },
  presets: [createPreset()],
};
