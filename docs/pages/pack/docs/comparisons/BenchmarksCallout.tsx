import Link from "next/link";
import Callout from "../../../../components/Callout";

export default function BenchmarksCallout() {
  return (
    <Callout type="info">
      Want to know more about Turbopack&apos;s benchmarking process and
      philosophy?{" "}
      <Link href="../benchmarks" className="nx-underline">
        Learn more about Turbopack&apos;s benchmarking suite.
      </Link>
    </Callout>
  );
}
