import { redirect } from 'next/navigation';
import Link from 'next/link';
import { getSession } from '../../lib/session';

export default async function Login() {
  const session = await getSession();
  if (session) {
    redirect(`/teams/${session.teamId}`);
  }
  return <Link href="/settings">Settings</Link>;
}
