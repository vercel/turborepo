import { openapi } from "@/lib/openapi";
import { shikiTheme } from "@/lib/shiki-theme";
import { createAPIPage } from "fumadocs-openapi/ui";
import client from "./api-page.client";

export const APIPage = createAPIPage(openapi, {
  client,
  shikiOptions: {
    themes: {
      light: shikiTheme,
      dark: shikiTheme
    }
  }
});
