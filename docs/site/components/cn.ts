export function cn(...classes: Array<string | undefined | boolean>) {
  return classes.filter(Boolean).join(" ");
}
