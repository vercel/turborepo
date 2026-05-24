import { createRoot } from "react-dom/client";
import "./style.css";
import { Header, Counter } from "@repo/ui";

const App = () => (
  <main>
    <p className="eyebrow">Rsbuild + Turborepo</p>
    <Header title="Web" />
    <p className="intro">
      A React app powered by Rsbuild and a shared workspace UI package.
    </p>
    <section className="card">
      <span>Counter from @repo/ui</span>
      <Counter />
    </section>
  </main>
);

createRoot(document.getElementById("root")!).render(<App />);
