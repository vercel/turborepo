import { relations } from "drizzle-orm";
import { roleTable, userRoleTable, userTable } from "./table.schema";

// Users and roles are linked many-to-many through `userRoleTable`.
export const userRelations = relations(userTable, ({ many }) => ({
  userRoles: many(userRoleTable),
}));

export const roleRelations = relations(roleTable, ({ many }) => ({
  userRoles: many(userRoleTable),
}));

export const userRoleRelations = relations(userRoleTable, ({ one }) => ({
  user: one(userTable, {
    fields: [userRoleTable.userId],
    references: [userTable.id],
  }),
  role: one(roleTable, {
    fields: [userRoleTable.roleId],
    references: [roleTable.id],
  }),
}));
