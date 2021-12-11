import { log } from "logger";
import { CounterButton } from "ui";

export default function Store() {
  log("Hey! This is Home.");
  return (
    <div>
      <h1>Store</h1>
      <CounterButton />
    </div>
  );
}
