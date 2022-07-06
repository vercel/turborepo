const colors = require("tailwindcss/colors");

module.exports = {
  content: [
    // app content
    `./**/*.{js,ts,jsx,tsx}`,
    // package content
    `../../packages/**/*.{js,ts,jsx,tsx}`,
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
