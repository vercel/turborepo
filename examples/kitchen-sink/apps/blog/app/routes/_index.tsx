import { CounterButton } from "@repo/ui/counter-button";
import { Link } from "@repo/ui/link";

export default function Index(): JSX.Element {
  return (
    <div className="container">
      <h1 className="title">
        Blog <br />
        <span>Kitchen Sink</span>
      </h1>
      <CounterButton />
      <p className="description">
        Built With <Link href="https://turbo.build/repo">Turborepo</Link>
        {" & "}
        <Link href="https://remix.run/">Remix</Link>
      </p>
    </div>
  );
}
