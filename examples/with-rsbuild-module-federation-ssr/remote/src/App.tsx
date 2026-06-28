import { useEffect } from "react";
import { SharedBadge } from "@mf-rsbuild-ssr-example/shared-ui";
import "./App.css";
import Counter from "./components/Counter";

export default () => {
  useEffect(() => {
    console.log("Remote useEffect");
  }, []);

  return (
    <section className="remote">
      <div className="remote-card">
        <div className="eyebrow">Remote · Federated SSR</div>
        <div className="title">Payload bay</div>
        <p className="copy">
          This component is exposed from the remote and rendered by the SSR host.
        </p>
        <Counter />
        <SharedBadge label="shared ui from remote" />
      </div>
    </section>
  );
};
