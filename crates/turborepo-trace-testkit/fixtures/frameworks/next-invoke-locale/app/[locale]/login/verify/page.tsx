import { redirect } from 'next/navigation';
import { pendingEmail } from '../../../../lib/loaders';

export default async function Verify() {
  const email = await pendingEmail();
  if (!email) {
    redirect('/login');
  }
  return <p>Enter the code we sent to {email}.</p>;
}
