import React from "react";
import { DocsThemeConfig } from "./types";

export const ThemeConfigContext = React.createContext<DocsThemeConfig>({});
export const useConfig = () => React.useContext(ThemeConfigContext);
