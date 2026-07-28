import { sql } from "drizzle-orm";
import * as p from "drizzle-orm/pg-core";

const idUuid = p.uuid("id").primaryKey().default(sql`gen_random_uuid()`);
const createdAt = p
  .timestamp("created_at", { withTimezone: true })
  .notNull()
  .defaultNow();
const updatedAt = p
  .timestamp("updated_at", { withTimezone: true })
  .notNull()
  .defaultNow()
  .$onUpdate(() => new Date());

const Audit = {
  id: idUuid,
  createdAt,
  updatedAt,
};

export const userTable = p.pgTable("sys_user", {
  ...Audit,
  name: p.text("name").notNull(),
  email: p.text("email").notNull().unique(),
  emailVerified: p.boolean("email_verified").default(false),
  image: p.text("image"),


  role: p.varchar("role", { length: 50 }),
  banned: p.boolean("banned").default(false),
  banReason: p.text("ban_reason"),
  banExpires: p.timestamp("ban_expire_at", { withTimezone: true }),

  phone: p.text("phone"),
  whatsapp: p.varchar("whatsapp", { length: 50 }),
  position: p.varchar("position", { length: 100 }),

  isActive: p.boolean("is_active").default(true),
  isSuperAdmin: p.boolean("is_super_admin").default(false),
});

export const roleTable = p.pgTable("sys_role", {
  id: idUuid,
  name: p.text("name").notNull(),
  description: p.text("description"),
  type: p
    .varchar("type", { enum: ["system", "custom"] })
    .default("custom")
    .notNull(),
  priority: p.integer("priority").default(0).notNull(),
});

export const userRoleTable = p.pgTable(
  "sys_user_role",
  {
    userId: p
      .uuid("user_id")
      .notNull()
      .unique()
      .references(() => userTable.id, { onDelete: "cascade" }),
    roleId: p
      .uuid("role_id")
      .notNull()
      .references(() => roleTable.id, { onDelete: "restrict" }),
  },
  (t) => [p.primaryKey({ columns: [t.userId, t.roleId] })]
);