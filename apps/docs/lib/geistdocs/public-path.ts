import { getPublicPath } from "@vercel/geistdocs/config";
import { localizeHref } from "@vercel/geistdocs/localize-href";
import { config } from "./config";

export const getLocalizedPath = (lang: string | undefined, path: string) => {
  const defaultLanguage = config.defaultLanguage ?? "en";
  const localizedPath = localizeHref(
    path,
    lang ?? defaultLanguage,
    defaultLanguage
  );

  return getPublicPath(localizedPath, config.basePath);
};
