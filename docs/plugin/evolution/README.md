# 脚本驱动的运行时演进

`bootstrap.rhai` 和 `bootstrap.ts` 实现同一策略：读取活动代数、确认所需宿主能力、请求模型填写源码、保存已验证候选、按原代数提交。`observe.rhai` 和 `observe.ts` 根据指定版本、激活代数及增强路径的完成计数决定是否恢复先前版本。策略是普通增强，可以通过相同流程更换语言或更新自身；执行中的策略始终使用旧版本快照。

这套策略要求 `EvolutionPlugin`，以及提供 `/v1/model/enhancement/source` 的模型插件。默认宿主组合包含演进机制；精简宿主可以省略。平台没有内置模型客户端或模型密钥，模型能力可以由原生 Provider 或已授权的 MCP/gRPC 导入提供。[Agent 的 MCP 模型适配器](https://github.com/org-aio/aio-plugin-agent/tree/main/backend/src/generated/enhancement) 已提供源码生成工具，复用加密模型配置、Responses 引擎、受控出站和共享配额；部署方明确绑定租户及模型额度所有者，不转发任意用户身份。

产品集成验收已使用实际 Agent 进程与 MCP 导入完成双语言、MCP/gRPC 入口、激活、观察回滚、策略自更新、提交前撤权和重启恢复。只有最终上游模型使用受控 SSE 替身；真实模型生成质量、生产绑定、无人值守调度和完整失败审计尚未完成验收。

## 安装策略

`contracts.json` 是两种语言共用的声明与有序调用样例。样例中的模型响应只是验证记录，发布时不会请求模型或产生外部副作用。在本目录执行以下命令，将所选源码和契约合成一个可由现有管理员 API 安装的包：

```sh
python3 - <<'PY'
import json
from pathlib import Path
language = 'typescript'  # 改为 rhai 即选择 Rhai 实现
suffix = 'ts' if language == 'typescript' else 'rhai'
contracts = json.loads(Path('contracts.json').read_text(encoding='utf-8'))
entries = []
for name in ('bootstrap', 'observe'):
    entry = contracts[name]
    entry['language'] = language
    entry['source'] = Path(name + '.' + suffix).read_text(encoding='utf-8')
    entries.append(entry)
Path('/tmp/aio-evolution.json').write_text(
    json.dumps({'schema_version': 1, 'enhancements': entries}, ensure_ascii=False),
    encoding='utf-8',
)
PY
```

用当前租户管理会话向 `POST /api/runtime/enhancements/revisions` 提交该包，再将返回摘要和当前 `expected_generation` 提交到 `PUT /api/runtime/enhancements/active`。已有活动包时，应把这两项加入现有包，保留旧路径和样例，不能直接覆盖成只有策略的包。

角色需要 `plugin:manage` 和每项策略声明的 `runtime:<path>` 权限。`plugin:manage` 本身不授予宿主能力。授权插件还须实现 `AuthorizationProvider::session_has_permissions`，从当前会话和角色关系查询权限；未实现时演进能力默认拒绝。Wasm/Sidecar 还需在自身清单声明对应 runtime 路径，并携带真实交互上下文。后台进程和脚本输入不能伪造用户授权。

## 请求与模型契约

调用 `/v1/evolution/bootstrap` 时提供目标路径、语言、依赖和真实验收样例。例如通过 gRPC `Invoke` 或 MCP `v1.evolution.bootstrap` 的 `input` 传入：

```json
{
  "expected_generation": 1,
  "requirement": "将输入的数字乘以二，返回数字",
  "target": {
    "path": "/v1/math/double",
    "language": "rhai",
    "source": "",
    "calls": [],
    "capabilities": [],
    "examples": [{"input": 3, "output": 6}]
  }
}
```

模型能力接收 `{requirement, target, available}`，返回 `{source}`。`available` 是请求依赖中已装配且调用者获准使用的能力契约；模型不能决定 tenant、actor、generation，也不能修改目标路径、语言、依赖或验收样例。缺少原生机制时策略在请求模型前失败，不让模型虚构宿主接口。生成源码仍须编译、通过新旧样例和原子提交。

模型调用的截止时间属于整条增强链，默认总等待不超过 30 秒；没有自动重试。大型模型生成应拆成有界任务并由策略查询结果，目前示例只适合这个时间预算内的生成。单次能力输入输出最多 64 KiB，即使管理员 HTTP 包接口接受 256 KiB，脚本也不能绕过自身的传输限额。

## 观察与恢复

调用 `/v1/evolution/observe` 提供 `{revision, expected_generation, path, previous_revision, minimum_completed, failure_ratio}`。策略只在指定版本仍活动、完成样本足够且失败比例达到阈值时回滚。观察计数绑定激活代数，同一摘要重新激活不会继承此前故障。运行失败也可能来自输入、权限或外部服务，生产策略应选择专门验收路径和适合业务的阈值。

`started - succeeded - failed` 包括尚未完成、取消和进程中断，不能作为失败数。统计只记录路径、计数和毫秒时间，不记录输入输出或凭据。每次正式调用增加开始、完成两次数据库写入；开始计数失败时不执行脚本，完成计数保存失败时明确返回错误，不自动重试业务副作用。

候选编译、行锁等待之后会再次核验会话、管理权限和对应 runtime 权限；撤权回滚整个提交事务。激活失败保留旧版本。代数冲突要求重新审阅，不能重试覆盖新提交。回滚仅恢复当前租户曾激活且依赖仍可用的版本。

`/v1/runtime/enhancements/history` 返回最近 50 次指针提交及真实操作人，候选作者和不可变源码沿用现有版本记录。取消可能发生在数据库提交响应之前，调用方必须读取 head/history 确认结果；不能把超时视为事务必定未提交。

`protocols::tests::evolution` 使用真实 PostgreSQL、MCP SDK 和 gRPC 客户端验证双语言升级、策略自更新、观察回滚、并发版本条件、撤权、取消和插件缺省。平台单元集成测试替换模型生成器；Agent 的集成测试验证实际模型适配器与 Responses 引擎，只替换最终上游 HTTP/SSE 服务。两者都不能据此宣称真实模型自举已上线。
