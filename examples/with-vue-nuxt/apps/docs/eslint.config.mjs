import turboConfig from "eslint-config-turbo/flat";
import withNuxt from "./.nuxt/eslint.config.mjs";

export default withNuxt(...turboConfig);
