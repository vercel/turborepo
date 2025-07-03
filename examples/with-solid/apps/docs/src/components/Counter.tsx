import { createSignal } from "solid-js";
import "./Counter.css";
import { Button } from "@repo/ui";

export default function Counter() {
  const [count, setCount] = createSignal(0);
  return (
    <Button
      class=" bg-black text-white w-fit"
      onClick={() => setCount(count() + 1)}
      type="button">
      Docs Clicks: {count()}
    </Button>
  );
}
