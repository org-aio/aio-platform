# 计费与钱包设计

本文定义 AIO 平台的账户余额、充值、用量计费和智能体订阅边界。设计目标是先统一“谁在什么维度被收费、以什么证据收费、失败如何回滚”，再让文件、智能体和后续插件共用同一套账本。

当前仓库只包含本设计文档，钱包与计费实现尚未落地；身份/账户插件仍分别托管在 `aio-plugin-identity` 和 `aio-plugin-account`，落地时需要先确认这两个仓库的工作树与远端状态。

## 计费对象与维度

计费主体是**租户（workspace）**，不是单个用户。用户属于租户，租户持有钱包余额与订阅；同一租户内多个用户共享额度。个人资料页展示的是“当前租户钱包”，文案上明确写成工作区钱包，避免用户误以为余额私有。

- 主体：`tenant_id`（来自登录会话，请求不能传入）。
- 记账发起人：`user_id`，仅用于审计与归因，不参与扣费归属。
- 资源维度：按插件定义，例如文件插件的存储与图床流量、智能体的 token 与工具调用。

## 三类收入

1. **钱包充值**：一次性充值到租户余额，用于按量扣费。
2. **智能体订阅**：例如 `60$/月` 的套餐，按月授予额度或功能集合，不直接等于余额。
3. **后续按量增值**：存储、图床外链流量、第三方工具调用等，从余额或套餐额度扣减。

钱包与订阅是两套账：余额是通用货币，订阅是带有效期和范围的权利。一个租户可以只有余额、只有订阅，或两者都有。

## 数据模型

所有表都以 `tenant_id` 为隔离键，主键使用服务端生成的 UUID/文本 ID，时间统一存 `TIMESTAMPTZ`。

### 钱包

```sql
CREATE TABLE billing_wallets (
    tenant_id TEXT PRIMARY KEY,
    currency TEXT NOT NULL DEFAULT 'USD',
    balance_micros BIGINT NOT NULL DEFAULT 0,   -- 以 1e-6 货币单位存储，避免浮点
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE billing_ledger (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    kind TEXT NOT NULL,            -- recharge | usage | refund | subscription_grant | adjustment
    amount_micros BIGINT NOT NULL, -- 正为入账，负为出账
    balance_after_micros BIGINT NOT NULL,
    reference TEXT,                -- 外部订单号 / 用量 ID，用于幂等
    description TEXT NOT NULL,
    created_by TEXT NOT NULL,      -- user_id 或 system
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX billing_ledger_reference_idx
    ON billing_ledger (tenant_id, kind, reference)
    WHERE reference IS NOT NULL;
```

账本是唯一事实来源，`billing_wallets.balance_micros` 是账本求和的物化快照。任何扣费都必须在同一事务里写账本并更新快照，禁止只改余额不记账。

### 充值订单

```sql
CREATE TABLE billing_orders (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    amount_micros BIGINT NOT NULL,
    currency TEXT NOT NULL DEFAULT 'USD',
    provider TEXT NOT NULL,        -- manual | stripe | alipay | wechat
    provider_order_id TEXT,
    status TEXT NOT NULL,          -- pending | paid | failed | refunded
    created_by TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    paid_at TIMESTAMPTZ
);
CREATE UNIQUE INDEX billing_orders_provider_idx
    ON billing_orders (provider, provider_order_id)
    WHERE provider_order_id IS NOT NULL;
```

充值流程：创建 `pending` 订单 → 支付渠道回调 → 校验签名与金额 → 事务内把订单置 `paid` 并写 `recharge` 账本。回调必须幂等，重复回调命中唯一索引后直接返回成功，不重复入账。

### 订阅

```sql
CREATE TABLE billing_plans (
    id TEXT PRIMARY KEY,
    code TEXT NOT NULL UNIQUE,        -- agent_pro_60
    title TEXT NOT NULL,
    price_micros BIGINT NOT NULL,
    currency TEXT NOT NULL DEFAULT 'USD',
    interval TEXT NOT NULL,           -- month | year
    included_usage JSONB NOT NULL,    -- {"agent_tokens": 20000000}
    features JSONB NOT NULL,
    active BOOLEAN NOT NULL DEFAULT true
);

CREATE TABLE billing_subscriptions (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    plan_id TEXT NOT NULL,
    status TEXT NOT NULL,             -- active | past_due | canceled | expired
    period_start TIMESTAMPTZ NOT NULL,
    period_end TIMESTAMPTZ NOT NULL,
    auto_renew BOOLEAN NOT NULL DEFAULT true,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX billing_subscriptions_tenant_idx
    ON billing_subscriptions (tenant_id, status, period_end DESC);
```

每个计费周期开始时，把套餐 `included_usage` 写入周期额度表；额度按周期重置，不跨期累计。

```sql
CREATE TABLE billing_usage_grants (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    subscription_id TEXT,
    resource TEXT NOT NULL,           -- agent_tokens | storage_bytes | image_egress
    quantity BIGINT NOT NULL,         -- 授予量
    consumed BIGINT NOT NULL DEFAULT 0,
    period_start TIMESTAMPTZ NOT NULL,
    period_end TIMESTAMPTZ NOT NULL
);
```

扣费顺序固定为：先扣当前周期套餐额度，额度用尽后再扣钱包余额。这个顺序写进实现，不允许各插件自行决定。

### 用量事件

```sql
CREATE TABLE billing_usage_events (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    source_id TEXT NOT NULL,          -- 插件标识，例如 file / agent
    resource TEXT NOT NULL,
    quantity BIGINT NOT NULL,
    unit_price_micros BIGINT NOT NULL,
    amount_micros BIGINT NOT NULL,
    idempotency_key TEXT NOT NULL,
    occurred_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX billing_usage_events_idempotency_idx
    ON billing_usage_events (tenant_id, source_id, idempotency_key);
```

`idempotency_key` 由计费方提供，例如智能体用 `message_id`，文件插件用 `file_id + 事件类型`。重复上报命中唯一索引即视为已处理，返回既有结果。

## 计费流程

统一入口是一个内部计费 Service（Dill 注册），插件不直接改钱包表。

```
插件动作 → 采集用量 → 计费 Service.meter() → 扣套餐额度 → 扣余额 → 写账本/用量事件 → 返回剩余额度
```

- **预检**：昂贵或可中断的动作（发起智能体生成、上传大文件）先调用 `check(resource, estimated_quantity)`，余额与额度都不足时直接拒绝，避免先产生成本再欠费。
- **后结算**：智能体按实际 token 在生成完成后结算，用预检额度做上限，超出部分从余额补扣，余额不足则记录欠费事件并限制后续请求。
- **失败回滚**：动作失败不产生用量事件；已预授权的额度释放。账本只记录真实发生的用量。

## 智能体计费

智能体已经是 `process` 插件，按 token 计费的数据点现成：`agent_messages.tokens` 在生成完成时写入。计费接入点放在这条写入附近，而不是另起一条链路。

- 资源维度：`agent_tokens`，按上游 `usage.total_tokens` 结算。
- 价格：按模型档位配置单价，存 `billing_plans`/独立价目表，不在代码里写死。
- 幂等键：`assistant message id`，保证重试或恢复不重复扣费。
- 订阅：`agent_pro_60` 这类套餐按月授予固定 token 额度与功能集合；额度用尽后回落到钱包按量计费，或按配置直接拒绝。

`60$/月` 套餐建议表达为“每月包含 N token 额度 + 全部智能体功能”，额度数字随模型成本调整，套餐价格保持稳定，避免把单价写死在产品文案里。

## API 边界

钱包与充值属于身份/账户侧，建议落在 `aio-plugin-identity`（服务端）与个人资料页（客户端）：

| 方法 | 路径 | 说明 |
| --- | --- | --- |
| `GET` | `/api/billing/wallet` | 当前租户余额与当前周期额度 |
| `GET` | `/api/billing/ledger?cursor=` | 账本分页 |
| `POST` | `/api/billing/orders` | 创建充值订单 |
| `POST` | `/api/billing/orders/{id}/confirm` | 支付回调 / 人工确认 |
| `GET` | `/api/billing/subscription` | 当前订阅与套餐 |
| `POST` | `/api/billing/subscription` | 订阅或切换套餐 |

个人资料页新增“钱包”区块：余额、当前套餐、额度进度、充值入口、最近账本。充值 Dialog 只做金额与渠道选择，不展示内部订单 ID。

## 权限与安全

- 余额与账本只对租户成员可见；充值、订阅变更需要独立权限，例如 `billing:manage`，不能复用 `file:manage`。
- 金额一律用整数微单位，禁止浮点；展示层再格式化。
- 支付回调必须校验签名、金额、币种与订单状态，任何不匹配都拒绝。
- 不在日志、账本描述或前端暴露渠道密钥、回调原文、完整账号状态。
- 所有写操作以 `tenant_id + idempotency_key` 去重，回调与重试都必须幂等。

## 落地顺序

1. ✅ 身份插件落地钱包、账本、用量事件表与计费 Service。
2. ✅ 个人资料页接入钱包展示、充值订单、订阅与账本分页。
3. ✅ 设置中心接入支付宝渠道配置，服务端按 RSA2 生成收银台地址并在异步通知验签入账。
4. ✅ 智能体在 `agent_messages.tokens` 写入点经宿主 broker `/meter` 上报 `agent_tokens`。
5. ⬜ 文件插件接入存储与图床流量计费，复用同一 Service。
6. ⬜ 套餐管理页面与人工调账工具。

### 支付渠道

支付宝配置位于“设置中心 → 支付”，字段包括 `app_id`、网关、异步通知地址、同步跳转地址、商户 UID、支付宝公钥和应用私钥。应用私钥使用 `AIO_BILLING_SECRET_KEY`（32 字节 Base64）以 AES-256-GCM 加密后入库，接口只返回 `has_private_key`。启用渠道前必须同时配置应用私钥和支付宝公钥。

异步通知地址应指向 `https://<站点>/api/billing/alipay/notify`。回调必须通过 RSA2 验签，并校验 `app_id` 与订单金额；重复通知按订单状态幂等。未知订单号返回 `success` 以避免无限重试。

### 进程插件计量

网络隔离的进程插件不能直连身份插件，只能通过宿主 broker 的 `/meter` 上报。宿主通过 `IdentityProvider::meter` 结算，`aio-idea` 直接把用量交给身份插件的 `meter_resource`，价格取自 `billing_prices` 表。未接入计费的宿主返回 204，调用方不应视为失败。

每一步都以真实数据库和浏览器流程验收，不以编译通过或单测代替充值、扣费、退款的端到端验证。
