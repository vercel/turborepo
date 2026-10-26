const HEADING = "Storefront catalog";

function render(schema) {
  const lines = schema.products.map((product) =>
    [product.id, product.name, (product.price / 100).toFixed(2)].join("  ")
  );
  return [HEADING, "schema " + schema.version, ...lines, ""].join("\n");
}

module.exports = { render };
