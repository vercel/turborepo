module.exports = {
  plugins: [
    require("postcss-pxtorem")({
      rootValue: 16,
      propList: ["font-size"],
    }),
  ],
};
