import Link from 'next/link';

export function SiteHeader() {
  return (
    <nav>
      <Link href="/">Home</Link>
      <Link href="/pricing">Pricing</Link>
    </nav>
  );
}
