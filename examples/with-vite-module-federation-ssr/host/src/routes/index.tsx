import { SharedBadge } from "@mf-vite-ssr-example/shared-ui";
import { createFileRoute } from "@tanstack/react-router";
import { Component, lazy, Suspense, useEffect, type ReactNode } from "react";
import { of, tap } from "rxjs";
import "../App.css";
import Counter from "../components/Counter";

const Remote = lazy(() => import("remote/remote-app"));

// Pre-load remote modules before render so the MF runtime resolves them
// server-side via ssrEntryLoader. Without this, React.lazy returns the
// dev proxy synchronously — the module is null during SSR and renders nothing.
export const Route = createFileRoute("/")({
  loader: () => import("remote/remote-app").then(() => null),
  component: IndexPage,
});

class RemoteErrorBoundary extends Component<
  { children: ReactNode },
  { error: Error | null }
> {
  state = { error: null };

  static getDerivedStateFromError(error: Error) {
    return { error };
  }

  render() {
    if (this.state.error) {
      return (
        <div className="loading">
          Remote failed to load: {(this.state.error as Error).message}
        </div>
      );
    }
    return this.props.children;
  }
}

function IndexPage() {
  useEffect(() => {
    of("emit")
      .pipe(tap(() => console.log("I'm RxJs from host")))
      .subscribe();
  }, []);

  return (
    <main className="shell">
      <section className="host">
        <div className="card host-card">
          <div className="eyebrow">Host · Vite SSR</div>
          <div className="title">Flight deck</div>
          <p className="copy">
            This shell is server-rendered and loads the remote module at runtime.
          </p>
          <Counter />
          <SharedBadge label="shared ui from host" />
        </div>
      </section>

      <RemoteErrorBoundary>
        <Suspense fallback={<div className="loading">Loading remote...</div>}>
          <Remote />
        </Suspense>
      </RemoteErrorBoundary>
    </main>
  );
}
