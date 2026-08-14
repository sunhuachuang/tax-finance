# Finance：AI 辅助报税 → 个人财务 Agent

> 本文档记录项目的出发点、终局愿景、边界约束和推进顺序，供后续实现时参考。
> 创建于 2026-07-31。重大方向调整时更新本文档，而不是另开新文档。

## 出发点

用 AI 自动化个人财务中重复、易错、依赖翻找记录的部分：

1. **近期**：辅助报税软件。上传 invoice / receipt / 银行流水，AI 自动扫描提取并记账；报税时从账本生成申报所需数据（GST return、IR3）。地区先做新西兰。
2. **中期**：开放 MCP 给 crabtalk 自动调取和使用；可能做一个 app 交互。
3. **终局**：个人财务 agent——观察市场、掌握各渠道资产情况、给出分析建议、突发事件及时提醒。

## 不可动摇的架构原则

这两条已在 `crates/taxcore` 中确立，所有后续模块必须遵守：

1. **模型报告数字，但从不计算数字。** AI 负责提取、分类、解读；一切算术和税务计算走确定性代码。提取结果必须经过 `validate` 的算术校验。延伸到 agent 阶段即为：**模型解读和提醒，但不下指令、不自动执行**（链上白名单操作除外，见安全层）。
2. **规则是数据，不是代码。** 税率规则放 `rules/<jurisdiction>/<year>.yaml`，精确有理数，版本化，只增不改——已申报年份必须能在多年后复算出相同数字。
3. **一切有出处（provenance）。** 每条账、每个申报数字、未来每条建议和告警，都能追溯到原始文档、链上查询或数据源。账本 append-only，只冲销不删改。IRD 要求记录保存 7 年。

## 边界约束（安全与合规）

- **敏感渠道先人工**：银行、券商等渠道的数据获取和操作走人工/手动导入，不接银行 API、不做爬虫、不托管任何银行凭证。"手动"指人导出文件，提取和记账仍由 AI 管道自动完成。
- **只有链上资产允许自动化操作**，且必须先建成安全策略层（见下文），agent 永远不触碰私钥。
- **合规定位**：产品是"准备申报数据 / 提供信息和分析的工具"，不是税务建议或投资建议。自用无合规问题；若未来开放给他人，注意 NZ 的 tax agent 和 regulated financial advice（FMA 牌照）都是受监管定位。
- **MCP 写权限收紧**：读操作（查账、查 GST 期、生成报表数据）可开放给 crabtalk；写操作只能创建 pending 记录，确认动作留给人。agent 不能修改已确认的账。

## 阶段一：报税闭环（当前）

目标用户场景：sole trader / contractor（GST 注册、provisional tax），即自己的场景。先端到端跑通自己这条链路，再泛化。

已有（阶段一代码全部落地，2026-08-01）：

- `crates/taxcore` —— domain model（money、ledger、document、gst、taxyear、provenance、bank 对账）
- `rules/nz/2025-26.yaml` —— NZ 税年规则
- `crates/taxstore` —— SQLite append-only 存储。触发器级防篡改；分录与 provenance 同事务落库；extraction 版本化；文档状态机白名单。
- `crates/taxingest` —— ingestion pipeline：内容寻址存文件去重 → record_reading（算错必进 review、置信度只降不升）→ propose_draft（GST 确定性计算，永远是 Draft）→ review 队列（approve/reject，拒绝作废不删）→ 银行流水匹配候选。
- `crates/taxreturn` —— GST101 各 box + IR3 汇总，每行 ReturnLine 带 provenance 且 verify()。box 8/12 用逐票存储 GST 加总，另附 3/23 公式对照。box 9/13 调整项暂为零占位；IR3 未做法定扣除调整（娱乐 50% 等），输出里有明确 note。
- `crates/taxmcp` —— MCP server（stdio JSON-RPC）。读全开；写只造 pending；approve/post/reverse/void 不存在于工具表。

阶段一收尾剩余：

1. **端到端真实数据验证**：跑 `cargo test`，然后用自己的真实 invoice 走一遍全链路。
2. **人工确认界面**：approve/reject 目前只有 library API，需要一个最小 CLI（或复用 crabtalk 对话确认）。
3. **GST 调整项（box 9/13）与法定扣除调整**（娱乐 50%、home office、里程）建模。

## 阶段二：只读资产聚合

在 ledger 之外增加 `positions / holdings` 快照视图，形成"我的全部资产"。

- **链上资产（全自动，最先做）**：只需地址，只读。RPC / indexer 定时查余额和持仓，价格从行情 API 补。零私钥风险。
- **银行 / 券商 / KiwiSaver（半自动）**：人定期导出 CSV / PDF / 结单，复用阶段一的 ingestion pipeline（文档 → 提取 → 校验 → 审核）。
- 外币支持沿用 `ForeignAmount`。FIF、crypto 税务处理等 NZ 特有规则后置，但数据模型不设障碍。

## 阶段三：监控与告警

crabtalk cron agent 定时扫描持仓相关标的、宏观事件、链上异动，push notification 推送。

技术上是 monitor 循环 + 推送，真正要设计的是**信噪比**：

- 阈值分级：持仓波动 5% 与交易所暴雷不是一个级别；
- 去重：同一事件不重复轰炸；
- 相关性过滤：只提醒与实际持仓相关的事件。

时效性实事求是：分钟级轮询覆盖绝大多数场景且成本极低；秒级实时（websocket 常驻、mempool 监控）等有真实高频需求再上。

## 阶段四：建议层

AI 解读资产快照 + 市场数据，输出分析和提示（如集中度、现金占比、税务时点提醒）。只呈现信息与分析，决策留给人——这既是安全边界也是合规边界，保证自用到开放的路径是通的。

## 阶段五：链上执行 + 安全策略层

唯一允许 agent 执行操作的领域。骨架：

1. **私钥隔离**：签名放独立签名服务或硬件钱包；agent 只能提交结构化"交易意图"。
2. **策略引擎（确定性代码，不是 AI）**：合约 / 代币白名单、单笔与日累计限额、超阈值自动进入人工确认队列（push → 人确认 → 执行）。
3. **模拟先行**：每笔交易先在 fork 上 simulate，验证结果与意图一致（收到的代币、滑点范围）才允许签名。
4. **审计日志**：意图、决策依据、模拟结果、执行结果全部 append-only 记录，与 ledger 的 provenance 同一哲学。

## 推进顺序总览

```
阶段一  报税闭环          存储 → ingestion → 报表 → MCP        ← 当前
阶段二  只读资产聚合      链上只读全自动 + 文件导入半自动
阶段三  监控与告警        cron agent + push，信噪比设计
阶段四  建议层            AI 解读，只分析不执行
阶段五  链上执行          安全策略引擎，agent 不碰私钥
```

每一步都在前一步的数据上生长。自动化圈定在数据聚合、监控、解读这些 AI 可靠的环节；执行权留给人和确定性代码。
