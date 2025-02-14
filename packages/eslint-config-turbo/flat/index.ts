import plugin from "eslint-plugin-turbo";

const thing = [
  {
    plugins: {
      turbo: plugin,
    },
    rules: {
      "turbo/no-undeclared-env-vars": "error",
    },
  },
];

// eslint-disable-next-line import/no-default-export -- Matching old module.exports
export default thing;
