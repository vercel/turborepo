import { Nav } from '../../components/nav';

export default function LocaleLayout({ children }: { children: React.ReactNode }) {
  return (
    <html>
      <body>
        <Nav />
        {children}
      </body>
    </html>
  );
}
