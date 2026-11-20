import { Backend } from '../../components/backend';
import { Unknown } from '../../components/unknown';
import { Feed } from '../../components/feed';

export default function Settings() {
  return (
    <main>
      <Backend />
      <Unknown kind="a" />
      <Feed endpoint="/api/health" />
    </main>
  );
}
