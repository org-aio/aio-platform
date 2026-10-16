# 持久 Component 激活

`PersistentComponentSlot::open(ComponentDatabase::release_pool(), ...)` 创建绑定来源 UUID 和租户的 v2 执行槽。整包、实际能力、活动版本及变更历史保存在宿主 PostgreSQL schema；插件数据角色不可访问。宿主先通过 `install` 绑定数据库并完成受控增量迁移，再提供 `InvocationResources` 激活。

`activate` 在旧版本继续服务时完成候选校验与生命周期，随后排空本进程请求，并以 PostgreSQL 行锁排空其他宿主中的在途请求。活动记录提交后才交换本地实例。并发发布使用代数比较，失败保留已有版本。取消期间提交结果不确定时，旧实例拒绝后续调用，由 `restore` 恢复数据库中的版本。

启动时使用 `stored` 读取锁定整包及能力，再绑定对应资源并 `restore`；恢复仍须使用宿主当前允许的能力。旧摘要不能访问新版本。`deactivate` 持久撤销，`unload` 仅卸载本进程。迁移允许追加已校验的非破坏性建表/索引；不能用删除历史迁移进行回退。

这是 v2 运行库能力。正式产品的身份、导航、挂载票据和发布授权必须由产品接入，不把旧协议的发布接口作为适配器。
