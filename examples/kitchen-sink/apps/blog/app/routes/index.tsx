import { CounterButton, NewTabLink } from "ui";

export default function Index() {
  return (
    <div className="container">
      <h1 className="title">
        Blog <br />
        <span>Kitchen Sink</span>
      </h1>
      <CounterButton />
      <p className="description">
        Built With{" "}
        <NewTabLink href="https://turbo.build/repo">Turborepo</NewTabLink> +{" "}
        <NewTabLink href="https://remix.run/">Remix</NewTabLink>
      </p>
    </div>
  );
}
