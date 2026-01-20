import { Button, PriceDisplay } from "@repo/ui";
import { add, multiply } from "@repo/utils";

export function renderApp(): string {
  const total = add(10, multiply(5, 3));

  return `
    <div class="app">
      <h1>Welcome to the Web App</h1>
      ${Button({ label: "Get Started" })}
      ${PriceDisplay({ amount: total })}
    </div>
  `;
}

export function calculateTotal(items: number[]): number {
  return items.reduce((sum, item) => add(sum, item), 0);
}
