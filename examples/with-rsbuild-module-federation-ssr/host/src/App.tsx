import type { ComponentType } from "react";
import { SharedBadge } from "@mf-rsbuild-ssr-example/shared-ui";
import { Suspense, useEffect } from "react";
import { of, tap } from "rxjs";
import "./App.css";
import Counter from "./components/Counter";

type AppProps = {
  Remote: ComponentType;
};

export default function App({ Remote }: AppProps) {
  useEffect(() => {
    of("emit")
      .pipe(tap(() => console.log("I'm RxJs from host")))
      .subscribe();
  }, []);

  return (
    <main className="shell">
      <section className="host">
        <div className="card host-card">
          <div className="eyebrow">Host · Rsbuild SSR</div>
          <div className="title">Flight deck</div>
          <p className="copy">
            This shell is server-rendered and loads the remote module at runtime.
          </p>
          <Counter />
          <SharedBadge label="shared ui from host" />
        </div>
      </section>

      <Suspense fallback={<div className="loading">Loading remote...</div>}>
        <Remote />
      </Suspense>
    </main>
  );
}
