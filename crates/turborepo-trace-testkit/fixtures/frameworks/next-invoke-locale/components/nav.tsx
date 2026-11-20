import { Button } from 'ui-kit';
import { Link } from './link';

export function Nav() {
  return (
    <nav>
      <a href="/">Home</a>
      <Link href="/new">New poll</Link>
      <Button href="/login" variant="ghost">
        Sign in
      </Button>
      <Link href="#help">Help</Link>
    </nav>
  );
}
