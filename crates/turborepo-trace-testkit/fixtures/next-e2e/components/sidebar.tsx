import Link from 'next/link';

export function Sidebar() {
  return (
    <aside>
      <Link href="/dashboard">Overview</Link>
      <Link href="/dashboard/settings">Settings</Link>
    </aside>
  );
}
