import { slugify } from "core";

export function titleId(title: string): string {
  return `title-${slugify(title)}`;
}
