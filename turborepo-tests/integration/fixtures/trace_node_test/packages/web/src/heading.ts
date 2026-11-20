import { slug } from 'text/src/slug.ts';

export function heading(text: string): string {
  return `<h1 id="${slug(text)}">${text}</h1>`;
}
