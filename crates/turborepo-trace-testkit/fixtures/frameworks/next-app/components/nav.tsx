import Link from 'next/link';

export function Nav() {
  return <nav className="flex gap-2"><Link href="/about">About</Link><Link href={`/blog/${'new'}`}>Blog</Link></nav>;
}
