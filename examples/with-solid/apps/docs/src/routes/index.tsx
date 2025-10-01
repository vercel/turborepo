import Counter from "~/components/Counter";
import "./index.css";

export default function Home() {
  return (
    <main>
      <h1 class="text-hue">Hello Docs!</h1>
      <Counter />
      <p>
        Visit{" "}
        <a href="https://solidjs.com" target="_blank">
          solidjs.com
        </a>{" "}
        to learn how to build Solid apps.
      </p>
    </main>
  );
}
