import { defineRelations } from "drizzle-orm";
import * as schema from "./table.schema"

export const relations = defineRelations(schema, (r) => ({


  // [用户]：统一身份
  userTable: {
    // 权限关联
    // 多对多
    roles: r.many.roleTable({
      from: r.userTable.id.through(r.userRoleTable.userId),
      to: r.roleTable.id.through(r.userRoleTable.roleId),
    }),

  },
}))