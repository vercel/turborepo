const { renderButton } = require("@repo/ui");

exports.renderPage = () =>
  `<main><h1>Storefront</h1>${renderButton("Browse catalog")}</main>\n`;
