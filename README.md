# Finance — AI 辅助报税引擎（NZ）

AI 辅助报税 → 个人财务 agent。项目愿景、阶段规划和架构原则见 [ROADMAP.md](ROADMAP.md)。
本文档只讲一件事：**后端怎么构建、启动和运行**。

核心原则（会影响你使用方式的部分）：

- **模型报告数字，从不计算数字** —— AI 只做提取和分类，一切算术走确定性代码并经 `validate` 校验。
- **账本 append-only** —— 只冲销不删改，SQLite 触发器级防篡改。
- **写操作只造 pending** —— MCP 层没有 approve/post/reverse/void 工具；确认动作只存在于 taxweb 面板（人点击）。

## 工作区布局

| Crate | 类型 | 职责 |
|---|---|---|
| `taxcore` | 库 | domain model：money、ledger、document、GST、税年、provenance、银行对账 |
| `taxrules` | 库 | 税率规则加载与校验（规则是数据：`rules/<辖区>/<税年>.yaml`） |
| `taxstore` | 库 | SQLite append-only 存储，触发器防篡改 |
| `taxingest` | 库 | ingestion pipeline：存文件去重 → 提取记录 → 草稿分录 → review 队列 → 银行匹配 |
| `taxreturn` | 库 | GST101 各 box + IR3 汇总，每行带 provenance 且可 `verify()` |
| `taxmcp` | **二进制** | MCP server（stdio JSON-RPC），给 crabtalk / Claude 等 agent 用 |
| `taxweb` | **二进制** | 本地面板 + 人工确认闸口（approve/reject 只在这里），只绑 127.0.0.1 |

## 构建

需要支持 edition 2024 的 Rust（1.85+）。

```bash
cd finance
cargo build --release
```

产物：`target/release/taxmcp` 和 `target/release/taxweb`。

运行测试：

```bash
cargo test
```

## 快速开始（demo 模式，不碰真实数据）

```bash
cargo run --release -p taxweb -- --demo
```

在系统临时目录造一个一次性账本、种入演示数据，然后打开 <http://127.0.0.1:5710/>。
demo 模式自动使用仓库内的 `rules/` 目录，适合先看一眼面板长什么样。

## 正式运行

### 1. 数据目录与规则文件

两个二进制共享同一套约定：

- **数据目录**（默认 `~/.taxdata`，可用 `--data-dir` 覆盖）：存 `ledger.db` 和内容寻址的原始文档。首次运行自动创建。
- **规则目录**（默认 `<数据目录>/rules`，可用 `--rules-dir` 覆盖）：按 `{rules_dir}/{辖区小写}/{税年}.yaml` 查找，例如 `rules/nz/2025-26.yaml`。

规则文件仓库里有，但默认位置不会自动有。二选一：

```bash
# 方式 A：把仓库规则拷到默认位置（升级规则时需重拷）
mkdir -p ~/.taxdata/rules
cp -R rules/nz ~/.taxdata/rules/

# 方式 B：启动时直接指向仓库（推荐，规则只增不改，跟着仓库走）
taxweb --rules-dir /path/to/finance/rules
```

### 2. 启动 taxweb（面板 + 人工确认）

```bash
cargo run --release -p taxweb                # 默认 ~/.taxdata，端口 5710
cargo run --release -p taxweb -- --port 5710 --data-dir ~/.taxdata --rules-dir ./rules
```

只监听 127.0.0.1（页面展示财务数据，不对外）。路由：

| 路由 | 方法 | 作用 |
|---|---|---|
| `/` | GET | 面板页面 |
| `/api/overview` | GET | 总览 |
| `/api/entries/` | GET | 分录列表 |
| `/api/gst` | GET | GST return 数据 |
| `/api/ir3` | GET | IR3 汇总 |
| `/api/entries/{id}/approve` | POST | **人工确认闸口**：批准草稿分录 |
| `/api/entries/{id}/reject` | POST | 拒绝草稿（作废不删） |

前台进程，`Ctrl-C` 停止。没有守护化——需要常驻时用 launchd/systemd 包一层，或等接入 crabup 服务管理。

### 3. 启动 taxmcp（MCP server）

stdio 传输、行分隔 JSON-RPC——**不是常驻服务**，由 MCP 客户端按需拉起，不需要手动启动：

```bash
taxmcp [--data-dir DIR] [--rules-dir DIR]   # 默认同上；手动跑只用于调试
```

接入 crabtalk：在其 MCP server 配置中登记命令（按 crabtalk 当前版本的 MCP 配置格式）：

```
command: /path/to/finance/target/release/taxmcp
args: ["--data-dir", "/Users/<you>/.taxdata", "--rules-dir", "/path/to/finance/rules"]
```

接入 Claude Code：

```bash
claude mcp add tax -- /path/to/finance/target/release/taxmcp \
  --data-dir ~/.taxdata --rules-dir /path/to/finance/rules
```

两个进程可以同时指向同一个数据目录：agent 经 taxmcp 录入，人开着 taxweb 审批。

### MCP 工具面（安全姿态靠"没有"来保证）

**读（全开）**：`list_documents`、`get_document`、`review_queue`、`list_entries`、`gst_return`、`ir3_summary`、`unreconciled_bank`、`propose_matches`

**写（只造 pending）**：`ingest_document`、`record_reading`、`propose_draft`、`import_bank_rows`

工具表里**不存在** approve/post/reverse/void，也不能强改文档状态——确认永远留给 taxweb 里的人。

## 端到端流程（阶段一闭环）

1. agent（或手动 JSON-RPC）经 `ingest_document` 上传 invoice/receipt —— 内容寻址存储，重复文件自动去重
2. 模型读取文档后经 `record_reading` 提交提取结果 —— 算术校验不过必进 review，置信度只降不升
3. `propose_draft` 生成草稿分录 —— GST 由确定性代码计算，永远是 Draft
4. 人在 taxweb 面板 approve / reject
5. `gst_return` / `ir3_summary` 从已确认账本生成申报数据，每行可溯源
