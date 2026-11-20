export function slug(text: string): string {
  return text.trim().toLowerCase().replace(/\s+/g, '-');
}
