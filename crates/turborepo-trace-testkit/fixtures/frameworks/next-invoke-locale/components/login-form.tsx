'use client';

import { useRouter } from 'next/navigation';

export function LoginForm() {
  const router = useRouter();
  return (
    <form
      onSubmit={(e) => {
        e.preventDefault();
        router.push('/login/verify');
      }}
    >
      <input name="email" />
      <button type="submit">Continue</button>
    </form>
  );
}
