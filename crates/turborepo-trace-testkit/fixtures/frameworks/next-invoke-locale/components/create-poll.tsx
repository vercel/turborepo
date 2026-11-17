'use client';

import { useRouter } from 'next/navigation';

async function createPoll(): Promise<{ id: string }> {
  return { id: crypto.randomUUID() };
}

export function CreatePoll() {
  const router = useRouter();
  return (
    <button
      onClick={async () => {
        const poll = await createPoll();
        router.push(`/poll/${poll.id}`);
      }}
    >
      Create poll
    </button>
  );
}
