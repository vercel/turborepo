import { TeamForm } from '../../../components/team-form';

export default function NewTeam() {
  async function subscribe(formData: FormData) {
    'use server';
    console.log(formData.get('email'));
  }

  return (
    <main>
      <TeamForm id="t1" />
      <form action={subscribe}>
        <input name="email" />
      </form>
    </main>
  );
}
