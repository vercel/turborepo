import { db } from "./client";

const DEFAULT_USERS = [
  {
    name: "Tim Apple",
    email: "tim@apple.com",
  },
];

try {
  await Promise.all(
    DEFAULT_USERS.map((user) =>
      db.orm.public.User.upsert({
        create: user,
        update: user,
        conflictOn: { email: user.email },
      }),
    ),
  );
} catch (error) {
  console.error(error);
  process.exitCode = 1;
} finally {
  await db.close();
}
