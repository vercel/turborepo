export const siteUrl = new URL("https://turborepo.dev");

export const absoluteUrl = (path: string) => new URL(path, siteUrl).toString();
