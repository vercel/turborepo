import { db } from "@repo/database";

export const dynamic = "force-dynamic";

export default async function IndexPage() {
  const users = await db.orm.public.User.all();

  return (
    <div>
      <h1>Hello World</h1>
      <pre>{JSON.stringify(users, null, 2)}</pre>
    </div>
  );
}
