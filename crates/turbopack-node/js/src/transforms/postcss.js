const {
  lazyPostCSS,
} = require("next/dist/build/webpack/config/blocks/css/index");
const { getSupportedBrowsers } = require("next/dist/build/utils");

module.exports = async (cssContent, name) => {
  const rootDir = process.cwd();
  const supportedBrowsers = getSupportedBrowsers(rootDir, true, {
    experimental: {
      legacyBrowsers: false,
    },
  });
  /**@type {{ postcssWithPlugins: import('postcss').Processor }} */
  const { postcssWithPlugins } = await lazyPostCSS(
    rootDir,
    supportedBrowsers,
    true
  );
  const { css, map } = await postcssWithPlugins.process(cssContent, {
    from: name,
    to: name,
    map: {
      inline: false,
    },
  });
  return { css, map };
};
