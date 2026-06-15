import { shikiTheme } from "@/lib/shiki-theme";
import { createOpenAPIPage } from "fumadocs-openapi/ui";

export const APIPage = createOpenAPIPage({
  shikiOptions: {
    themes: {
      light: shikiTheme,
      dark: shikiTheme
    }
  }
});
