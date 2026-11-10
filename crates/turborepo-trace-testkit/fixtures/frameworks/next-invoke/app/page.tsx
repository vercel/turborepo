import { Dash } from '../components/dash';
import { Jobs } from '../components/jobs';
import { Items } from '../components/items';

export default function Home() {
  return (
    <main>
      <Dash slug="acme" />
      <Jobs teamId="t1" enabled />
      <Items id="i1" />
    </main>
  );
}
