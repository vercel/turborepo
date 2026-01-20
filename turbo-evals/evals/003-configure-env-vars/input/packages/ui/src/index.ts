import { formatCurrency } from "@repo/utils";

export interface ButtonProps {
  label: string;
}

export function Button({ label }: ButtonProps): string {
  return `<button>${label}</button>`;
}

export interface PriceDisplayProps {
  amount: number;
  currency?: string;
}

export function PriceDisplay({ amount, currency }: PriceDisplayProps): string {
  const formatted = formatCurrency(amount, currency);
  return `<span class="price">${formatted}</span>`;
}
