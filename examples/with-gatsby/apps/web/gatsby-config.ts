import type { GatsbyConfig } from "gatsby";

const config: GatsbyConfig = {
  siteMetadata: {
    siteUrl: `https://www.yourdomain.tld`,
  },
  graphqlTypegen: true,
  plugins: [
    `gatsby-plugin-pnpm`,
    {
      resolve: `gatsby-plugin-compile-es6-packages`,
      options: {
        modules: [`ui`],
      },
    },
  ],
};

export default config;
