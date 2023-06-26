import type { PlopTypes } from "@turbo/gen";

// helpers
const dateToday = (): string =>
  new Date().toISOString().split("T")[0].replace(/-/g, "/");

const majorMinor = (version: string): string =>
  version.split(".").slice(0, 2).join(".");

export function init(plop: PlopTypes.NodePlopAPI): void {
  // add helpers for use in templates
  plop.setHelper("dateToday", dateToday);
  plop.setHelper("majorMinor", majorMinor);
}
