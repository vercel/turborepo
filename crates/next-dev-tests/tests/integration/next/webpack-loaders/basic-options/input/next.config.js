module.exports = {
  experimental: {
    turbopack: {
      loaders: {
        ".replace": [
          { loader: "replace-loader", options: { defaultExport: 3 } },
        ],
      },
    },
  },
};
