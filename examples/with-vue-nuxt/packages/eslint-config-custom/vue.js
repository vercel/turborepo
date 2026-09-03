import eslint from "@eslint/js";
import {
  vueTsConfigs,
  withVueTs,
} from "@vue/eslint-config-typescript";
import turboConfig from "eslint-config-turbo/flat";
import pluginVue from "eslint-plugin-vue";

export default withVueTs(
  { ignores: ["dist/**"] },
  eslint.configs.recommended,
  pluginVue.configs["flat/essential"],
  vueTsConfigs.recommended,
  turboConfig,
  {
    rules: {
      "vue/multi-word-component-names": "off",
    },
  },
);
