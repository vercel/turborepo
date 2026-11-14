import { UserPanel } from '../../../components/user-panel';
import { createCaller } from '../../../server/root';

export default async function UserPage() {
  // A server-side caller: no HTTP, the procedure runs in this render.
  const caller = createCaller({});
  const greeting = await caller.greeting();
  return (
    <main>
      {greeting}
      <UserPanel id="u1" />
    </main>
  );
}
