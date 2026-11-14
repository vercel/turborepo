import type { ReactNode } from 'react';
import { SiteHeader } from '../components/site-header';

export default function RootLayout({ children }: { children: ReactNode }) {
  return (
    <html lang="en">
      <body>
        <SiteHeader />
        {children}
      </body>
    </html>
  );
}
