import { SharedBadge } from "@mf-rsbuild-example/shared-ui";
import { Suspense, useEffect } from "react";
import Remote from "remote/remote-app";
import { of, tap } from "rxjs";
import "./App.css";
import Counter from "./components/Counter";

export default () => {
  useEffect(() => {
    of("emit")
      .pipe(tap(() => console.log("I'm RxJs from host")))
      .subscribe();
  }, []);

  return (
    <main className="shell">
      <section className="host">
        <div className="card host-card">
          <div className="eyebrow">Host · Rsbuild</div>
          <div className="title">Flight deck</div>
          <p className="copy">
            This app owns the shell and loads the remote module at runtime.
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
};
