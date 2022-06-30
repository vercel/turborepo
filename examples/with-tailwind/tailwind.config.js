const colors = require("tailwindcss/colors");
const path = require("path");

module.exports = {
  content: [
    `${path.resolve(__dirname, "apps/")}/**/*.{js,ts,jsx,tsx}`,
    `${path.resolve(__dirname, "packages/")}/**/*.{js,ts,jsx,tsx}`,
  ],
  theme: {
    extend: {
      colors: {
        brandblue: colors.blue[500],
        brandred: colors.red[500],
      },
    },
  },
  plugins: [],
};
