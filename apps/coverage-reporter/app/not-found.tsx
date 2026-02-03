import Link from "next/link";

export default function NotFound() {
  return (
    <main className="container">
      <div className="card" style={{ textAlign: "center" }}>
        <h1>404 - Not Found</h1>
        <p className="text-muted mb-2">
          The page or coverage report you&apos;re looking for doesn&apos;t
          exist.
        </p>
        <Link href="/">Return to Dashboard</Link>
      </div>
    </main>
  );
}
