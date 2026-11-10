import { redirect } from 'next/navigation';
import { cookies } from 'next/headers';

export type User = { name: string; role: 'admin' | 'member' };

async function currentUser(): Promise<User | undefined> {
  const session = (await cookies()).get('session')?.value;
  return session ? (JSON.parse(session) as User) : undefined;
}

/** The gate of the admin pages: a layout calls it to load its data. */
export async function loadAdmin(): Promise<User> {
  const user = await currentUser();
  if (!user) {
    redirect('/login');
  }
  if (user.role !== 'admin') {
    redirect('/admin-setup');
  }
  return user;
}

export async function pendingEmail(): Promise<string | undefined> {
  return (await cookies()).get('pending-email')?.value;
}
