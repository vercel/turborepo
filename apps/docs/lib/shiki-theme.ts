import { createCssVariablesTheme } from "shiki";

export const shikiTheme = createCssVariablesTheme({
  name: "css-variables",
  variablePrefix: "--shiki-",
  variableDefaults: {}
});
