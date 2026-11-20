export function formatPrice(cents: number): string {
  return `${(cents / 100).toFixed(2)}`;
}
