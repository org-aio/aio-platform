# 插件数据库绑定

专属数据库角色、加密凭据与有序迁移摘要保存在宿主管理 schema。重启重新打开同一绑定；迁移按摘要前缀验证并以表所有者权限执行。迁移失败回滚绑定和结构变更，不能默默重建数据空间。

结构迁移使用 SQL AST 统一校验：允许建表、建索引、新增列、添加约束及不带 CASCADE 的约束删除。跨 schema、外部位置、任意函数、删列、所有者变更及权限控制不开放；业务数据角色仍不能执行 DDL。
# 隔离进程连接

`ScopedDatabase::process_connection` 从同一个后端导出 `ProcessDatabaseConnection`。默认 PostgreSQL 后端仍先校验角色隔离，再生成专属 URL、TCP/Unix 上游与 `/database` 下的协议 socket；宿主不再解析 PostgreSQL 连接选项，也不会另向 Provider 索取凭据。管理数据库凭据不进入插件。
