import storybook from "eslint-plugin-storybook";
import react from "./react.js";

export default [...react, ...storybook.configs["flat/recommended"]];
