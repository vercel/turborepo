import type { ReactNode } from 'react';
import { Sidebar } from '../../components/sidebar';

export default function DashboardLayout({ children }: { children: ReactNode }) {
  return (
    <div>
      <Sidebar />
      <section>{children}</section>
    </div>
  );
}
