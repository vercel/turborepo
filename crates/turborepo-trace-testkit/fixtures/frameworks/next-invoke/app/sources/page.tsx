import { Computed, Last, Next, Tester } from '../../components/sources';

declare function pick(kind: string): string;

export default async function Sources({ searchParams }: { searchParams: Promise<{ u?: string }> }) {
  // A page's props are the request.
  const { u } = await searchParams;
  const res = u ? await fetch(u) : null;
  return (
    <main data-ok={res?.ok}>
      <Next />
      <Tester />
      <Computed client={{ query: pick('a') }} />
      <Last />
    </main>
  );
}
