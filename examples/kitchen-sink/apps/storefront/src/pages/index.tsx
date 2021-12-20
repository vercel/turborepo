import { log } from "logger";
import { CounterButton, Button } from "ui";

export default function Store() {
  log("Hey! This is Home.");
  return (
    <div>
      <h1>Store</h1>
      <CounterButton />
      <Button>Boop</Button>
    </div>
  );
}
