// Derived from https://github.com/vercel/next.js/blob/a19f04c5a1bbb27a9c7cbbc77a137e4a288abe1a/scripts/update-google-fonts.js
// Only includes generating the font-data.json, as TypeScript typings are maintained in the `next` npm package in the Next.js repo.

const fs = require("fs/promises");
const path = require("path");
const fetch = require("node-fetch");

(async () => {
  const { familyMetadataList } = await fetch(
    "https://fonts.google.com/metadata/fonts"
  ).then((r) => {
    if (r.status >= 400) {
      throw new Error(
        `Received bad status ${r.status} when retrieving font metadata`
      );
    }

    return r.json();
  });

  const fontData = {};
  for (let { family, fonts, axes, subsets } of familyMetadataList) {
    subsets = subsets.filter((subset) => subset !== "menu");
    const weights = new Set();
    const styles = new Set();

    for (const variant of Object.keys(fonts)) {
      if (variant.endsWith("i")) {
        styles.add("italic");
        weights.add(variant.slice(0, -1));
        continue;
      } else {
        styles.add("normal");
        weights.add(variant);
      }
    }

    const hasVariableFont = axes.length > 0;

    if (hasVariableFont) {
      weights.add("variable");
    }

    fontData[family] = {
      weights: [...weights],
      styles: [...styles],
      axes: hasVariableFont ? axes : undefined,
    };
  }

  await fs.writeFile(
    path.join(
      __dirname,
      "../crates/next-core/src/next_font_google/__generated__/font-data.json"
    ),
    JSON.stringify(fontData, null, 2)
  );
})();
