/** @type {import('tailwindcss').Config} */
export default {
  content: [
    "./pages/**/*.{js,ts,jsx,tsx,mdx}",
    "./components/**/*.{js,ts,jsx,tsx,mdx}",
    "./app/**/*.{js,ts,jsx,tsx,mdx}",
  ],
  theme: {
    extend: {
      colors: {
        background: "var(--background)",
        foreground: "var(--foreground)",
        "curious-blue": {
          50: "#f1f9fe",
          100: "#e3f1fb",
          200: "#c0e3f7",
          300: "#88cdf1",
          400: "#48b4e8",
          500: "#24a1de",
          600: "#137cb6",
          700: "#106394",
          800: "#12547a",
          900: "#144766",
          950: "#0d2d44",
        },
      },
      fontFamily: {
        bitcount: ["Audiowide", "monospace"],
      },
    },
  },
  plugins: [],
};
