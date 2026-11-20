import './globals.css';
import { Nav } from '../components/nav';

export default function RootLayout({ children }) {
  return <html><body><Nav />{children}</body></html>;
}
