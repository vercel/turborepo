import Counter from "~/components/Counter";
import "./index.css";
import { App } from "@repo/ui";

export default function Home() {
  return (
    <main>
      <h1 class="text-red-500">Hello Docs!</h1>
      <Counter />
      <p>
        Visit{" "}
        <a href="https://solidjs.com" target="_blank">
          solidjs.com
        </a>{" "}
        to learn how to build Solid apps.
      </p>
      <App />
    </main>
  );
}
