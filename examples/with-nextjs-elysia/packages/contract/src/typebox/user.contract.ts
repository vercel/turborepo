import { t } from "elysia";
import { userTable } from "../drizzle/table.schema";
import { spread, InferDTO } from "../utils";

/** [Auto-Generated] Do not edit this tag to keep updates. @generated */
export const UserInsertFields = spread(userTable, "insert");
/** [Auto-Generated] Do not edit this tag to keep updates. @generated */
export const UserFields = spread(userTable, "select");
export const UserContract = {
  /** [Auto-Generated] Do not edit this tag to keep updates. @generated */
  Response: t.Object({
    ...UserFields,
  }),

  Create: t.Object({
    name: UserFields.name,
    email: UserFields.email,
    phone: UserFields.phone,
    whatsapp: t.Optional(UserInsertFields.whatsapp),
    position: t.Optional(UserInsertFields.position),
    password: t.String(),
    deptId: t.String(),
    roleId: t.String(),
    isActive: t.Boolean(),
    masterCategoryIds: t.Optional(t.Array(t.String())),
  }),

  Update: t.Partial(
    t.Object({
      name: UserFields.name,
      email: UserFields.email,
      phone: UserFields.phone,
      whatsapp: t.Optional(UserInsertFields.whatsapp),
      position: t.Optional(UserInsertFields.position),
      password: t.String(),
      deptId: t.String(),
      roleId: t.String(),
      isActive: t.Boolean(),
      masterCategoryIds: t.Optional(t.Array(t.String())),
    })
  ),

  // Patch 请求 (部分更新)
  Patch: t.Partial(
    t.Object({
      ...t.Omit(t.Object(UserInsertFields), [
        "id",
        "createdAt",
        "updatedAt",
        "siteId",
      ]).properties,
    })
  ),

  ListQuery: t.Object({
    search: t.Optional(t.String()),
  }),
  /** [Auto-Generated] Do not edit this tag to keep updates. @generated */
  ListResponse: t.Object({
    data: t.Array(t.Object({ ...UserFields })),
    total: t.Number(),
  }),
};

export type UserContract = InferDTO<typeof UserContract>;
