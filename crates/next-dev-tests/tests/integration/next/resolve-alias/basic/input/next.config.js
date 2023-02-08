module.exports = {
  experimental: {
    turbopack: {
      resolveAlias: {
        foo: ["bar"],
        foo2: { browser: "bar" },
      },
    },
  },
};
