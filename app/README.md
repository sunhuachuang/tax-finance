# Finance App

桌面与移动客户端。选型理由、分层和不可动摇的约束见 [ARCHITECTURE.md](ARCHITECTURE.md)；
打包和发布见 [BUILD.md](BUILD.md)。本文档只讲两件事：**怎么配置，怎么跑起来**。

---

## 进程模型：桌面版只有一个进程

这一点值得先说清楚，因为它决定了后面所有配置项的含义。

`../service/` 里的 `taxcore` / `taxstore` / `taxrules` / `taxingest` / `taxreturn` 是**库**，
编译进了 app 二进制本身。app 在自己进程里直接打开 `ledger.db`——
**不走 HTTP、不开端口、不启动任何后台服务**。

```
桌面：  [ financeapp ]──直接读写──▶ ~/.taxdata/ledger.db
        └ taxcore / taxstore / taxingest / taxreturn 都在这个二进制里

移动：  [ financeapp ]──HTTP over Tailscale──▶ [ taxweb ]──▶ ledger.db
        （手机上不存副本，账本只有一份，在那台常驻 host 上）
```

`taxweb` 和 `taxmcp` 是**独立二进制，app 不会启动它们**：

| 二进制 | 谁启动 | 什么时候需要 |
|---|---|---|
| `taxweb` | 你手动跑在常驻 host 上 | 只有移动端 / 远程模式需要 |
| `taxmcp` | MCP 客户端（crabtalk / Claude Code）按需拉起 | 只有让外部 agent 访问账本时需要 |

所以桌面开发时，**只要跑 app 就够了**。

---

## 一次性准备

需要：

- Node 20+（开发用的是 24）
- 支持 edition 2024 的 Rust（1.85+）
- macOS 上需要 Xcode command line tools

```bash
cd app
npm install
```

`../service/` 必须与 `app/` 同级存在——`src-tauri/Cargo.toml` 用相对路径依赖它的 crates。
这是**编译期**依赖：编译完成后，二进制里已经包含了那些库的代码，运行时不再需要 `service/` 目录。

### 规则文件

税率规则是数据（`rules/<辖区>/<税年>.yaml`），app 运行时要读它们。
开发时最省事的做法是直接指向仓库：

```bash
export FINANCE_RULES_DIR="$(cd ../service && pwd)/rules"
```

不配的话默认找 `~/.taxdata/rules`，那个目录默认不存在——
总览页照常能用，但 GST 页和所得税页会报「no rule file for NZ ...」。

---

## 启动

```bash
npm run tauri dev
```

默认连本地账本 `~/.taxdata/ledger.db`，与 `taxweb` / `taxmcp` 共享同一份数据。
首次启动会自动建目录和空账本。

启动日志会告诉你当前状态：

```
financeapp: 数据来源 本地账本 /Users/you/.taxdata
financeapp: AI 助手未启用（未设置 API key）
financeapp: 前端已连接
```

- 第一行没出现 → 数据目录有问题
- `前端已连接` 没出现 → webview 里的脚本没跑起来（多半是 CSP 挡了）
- AI 助手未启用 → 点顶栏「设置」填一把 key，立刻生效

---

## 配置

**API key 和远程 host 在应用里的「设置」按钮里填**，不需要环境变量。
存在应用数据目录的 `settings.json`，权限 `0600`：

- macOS：`~/Library/Application Support/dev.neo.finance/settings.json`
- Linux：`~/.config/dev.neo.finance/settings.json`
- Windows：`%APPDATA%\dev.neo.finance\settings.json`

| 设置项 | 生效时机 |
|---|---|
| Claude API Key | **立刻**——存完助手就能用 |
| 远程 host | **重启后**——数据来源在启动时就定下来了 |

### 换一家模型服务（便宜很多，适合测试）

设置里的「模型服务」可以填**第三方的 Anthropic 兼容端点**。这些服务当初为了兼容
Claude Code 都实现了 `/anthropic/v1/messages`，**线上协议和 Anthropic 完全一样**，
所以流式、工具调用这些代码一行都不用改——只换地址和模型名。

对话框里有一排预设，点一下就填好：

| 服务 | 地址 | 模型 |
|---|---|---|
| Anthropic | 留空 | `claude-opus-5` |
| DeepSeek | `https://api.deepseek.com/anthropic` | `deepseek-v4-flash` |
| Kimi | `https://api.moonshot.ai/anthropic` | `kimi-k2-turbo-preview` |
| GLM | `https://api.z.ai/api/anthropic` | `glm-4.6` |
| MiniMax | `https://api.minimax.io/anthropic` | `MiniMax-M2` |

**换服务后 API key 也要换成对应服务的。** 第三方 key 不遵守 `sk-ant-` 命名，
所以 app 不按前缀判断——只要填了服务地址，key 一律按 `Authorization: Bearer` 发送
（这是这些端点的约定）。

第三方端点只实现了 Messages API 的公共部分，**`thinking` 和 `output_config.effort`
是 Anthropic 专有的**，发过去可能直接 400——所以填了服务地址时 app 会自动不发这两个字段。
代价是没有思考摘要，effort 旋钮也不起作用。

价格差得很远（每 100 万 token，输入/输出）：Opus 5 是 $5/$25，Haiku 4.5 是 $1/$5，
GLM-4.6 约 $0.43/$1.74，DeepSeek V4 Flash 约 $0.14/$0.28。
**只想省钱又不想换供应商，把模型名填成 `claude-haiku-4-5` 就够了**（地址仍留空）。

### 该填哪种凭证

到 `console.anthropic.com` 的 API Keys 页面创建一把 **API key**（`sk-ant-api03-…`）。

Anthropic 的凭证有三种，长得像但用法完全不同，填错会得到 401：

| 前缀 | 是什么 | 能用吗 |
|---|---|---|
| `sk-ant-api…` | Console 创建的 API key | ✅ 长期有效，走 `x-api-key` |
| `sk-ant-oat…` | OAuth access token（`ant auth login` 之类） | ⚠️ 能用，走 `Authorization: Bearer` + `oauth-2025-04-20`，但**会过期** |
| `sk-ant-sid…` | claude.ai 的网页会话 key | ❌ 不是 API 凭证 |

app 会按前缀自动认出类型并用对应的 header。401 时的报错也会针对具体类型给建议，
而不是干巴巴一句「API key is invalid」。

> 为什么是文件不是 keychain：`ledger.db` 里躺着 7 年的财务记录，本来就是明文。
> 只给 API key 上锁而账本敞着是安全表演。同一个信任级别、同一套文件权限，才是一致的。
> 哪天账本上了加密，这里再跟着上 keychain。

> key 在你输入的那一刻穿过页面一次，此后只在 Rust 侧使用——读设置时拿回来的是掩码，
> 不是原文。模型调用全在 Rust 侧，页面里任何东西（包括助手自己生成的布局）都读不到它。

### 环境变量（开发用，优先级更高）

设了就盖过设置文件里的值。打包后的应用读不到 shell 环境，所以这套只在开发时有用。

| 变量 | 作用 | 默认 |
|---|---|---|
| `FINANCE_DATA_DIR` | 账本与文档目录 | `~/.taxdata` |
| `FINANCE_RULES_DIR` | 规则 yaml 目录 | `<data-dir>/rules` |
| `FINANCE_HOST` | 远程模式 host | 看设置文件 |
| `ANTHROPIC_API_KEY` | Claude API key | 看设置文件 |

被环境变量盖住时设置界面会明说，免得出现「存了却不生效」的困惑。

一条完整的开发启动命令：

```bash
FINANCE_RULES_DIR="$(cd ../service && pwd)/rules" npm run tauri dev
```

---

## 拿一份演示数据

仓库里没有账本。想看有数据的界面，用 `taxweb` 的 demo 模式种一份：

```bash
cd ../service
cargo run -p taxweb -- --demo      # 输出里有 "demo data seeded under <目录>"
```

按 `Ctrl-C` 停掉它（只是借它来种数据，app 不需要它常驻），然后：

```bash
cd ../app
FINANCE_DATA_DIR=<上面那个目录> \
FINANCE_RULES_DIR="$(cd ../service && pwd)/rules" \
npm run tauri dev
```

那个目录在系统临时目录下，可能被清理。想留着就拷到别处。

---

## 远程模式（移动端的路径，桌面上也能试）

```bash
# 一个终端：当 host
cd ../service && cargo run -p taxweb -- --demo

# 另一个终端：app 当瘦客户端
cd app && FINANCE_HOST=http://127.0.0.1:5710 npm run tauri dev
```

远程模式下收文档也是通的：文件原样发给 host 的 `POST /api/documents`，
落在 host 的账本里，客户端不留副本。

真机上这条链路必须跑在 Tailscale / WireGuard 隧道里，**不要暴露到公网**。

---

## 移动端

首次需要初始化原生工程：

```bash
npm run tauri ios init
npm run tauri android init
npm run tauri ios dev
npm run tauri android dev
```

移动端没有本地账本，装机后必须在「设置」里填 host。

---

## AI 助手

改界面和问账本的主入口是右侧的对话框，不是属性面板。

模型是 `claude-opus-5`，effort 默认 `medium`——交互式对话里延迟是体感的一部分，
而这个模型在 medium 上已经很强。觉得改得不够聪明就往上调（`agent_set_effort`）。

没设 key 时界面照常显示，只是发不出去，顶部横幅有个「去设置」直接跳过去。

### 它能做什么，不能做什么

| 能 | 不能 |
|---|---|
| 读账本总览、GST101、IR3、文档与读数 | 批准 / 拒绝草稿 |
| 读当前布局、查可用块型 | 收文档、改文档状态 |
| 改布局（`apply_layout`） | 手打一个数字放进界面 |

「不能」那一列不是靠提示词劝住的，是**工具表里根本没有那些动作**——
和 `taxmcp` 同一个安全姿态。`apply_layout` 是唯一的写操作，改的是呈现不是账，
落库前必过块白名单校验（块型必须已注册、binding 的数据源必须合法、
span 必须在栅格内、人工确认闸口不能删）。

**助手的每一次改动都能撤销**——它改完布局后前端会拉回新文档并进撤销栈。

对话记录只在内存里，关掉应用就没了（Rust 侧的对话历史同理）。
开关面板不影响记录，正在流的回复也不会丢。

### 手动编辑还在

顶栏的「手动编辑」仍然可以拖拽和调属性。它和助手走同一份布局文档、
同一个撤销栈、同一套校验，所以不会出现「助手能做但手动做不到」这种分裂。

---

## 页面

出厂四页，全都是普通的布局文档，可以随便改：

| 页面 | 参数 | 内容 |
|---|---|---|
| 总览 | — | 四个计数、收文档、人工确认闸口、已入账分录表 |
| 文档 | 选中的文档（hidden） | 收文档 + 文档列表 + 详情（元信息、读数、校验问题、忽略/放回队列） |
| GST | 申报期内任一天 | 应缴 / 销项 / 进项三个数字 + GST101 各行，点行展开出处 |
| 所得税 | 税年 | 净利润 / 收入 / 支出 + IR3 汇总，点行展开出处 |

页面参数的**定义**存在布局文档里，**取值**是会话状态——换一次申报期不会写一版布局。

`control: "hidden"` 的参数不显示控件，由块自己写。**主从联动就靠它**：
文档列表块 `setParam("document", id)`，详情块 binding 里写 `$document`，
两个块互相不知道对方存在，中间人是布局文档。

### 关于「触发提取」

文档页**没有**「提取」按钮，是有意的。读文档要模型，读数经 MCP 的 `record_reading` 提交。
app 里放一个提取按钮就意味着 app 自己要接一个模型，那是另一个决定。
app 承接的是真正属于人的那两个决定：**忽略**和**放回待提取**。

---

## 测试

```bash
cd src-tauri && cargo test    # Rust：backend、ingest、布局持久化、agent
npm test                       # 前端：布局文档层的纯函数
npm run build                  # 前端类型检查随构建一起跑
```

有一个默认跳过的联调测试，需要一台在跑的 `taxweb`：

```bash
cd src-tauri && FINANCE_HOST=http://127.0.0.1:5710 cargo test -- --ignored
```

---

## 怎么加一个新块

块是这个 app 唯一的扩展点。新增一个：

1. 在 `src/blocks/` 写一个文件，导出组件，文件末尾调 `registerBlock({...})`；
2. 在 `src/blocks/index.ts` 里 import 它。

渲染、属性面板、schema 校验、序列化、拖拽、**以及 AI 助手能用它**，全部自动获得——
注册表是这些能力的唯一来源，启动时会推给 Rust 当白名单。

四类块的约束必须记住（完整理由见 ARCHITECTURE.md）：

- `kind: "data"`：数字**只能**来自 `binding`，属性面板不给填值的入口；
- `kind: "text"`：样式上必须看得出是注释，不能冒充数据；
- `kind: "action"`：可以写，但只造 pending，绝不直接产生账；
- `kind: "gate"`：必须 `locked: true`，布局怎么改都删不掉。

要接一个新的数据源（比如未来的持仓快照），在 `src/core/ipc.ts` 的 `DATA_SOURCES`
里加一项即可——binding 只能指向这张白名单。

---

## 已知的缺口

- **打包后的 app 还不能直接发**：规则文件没进 bundle。
  详见 [BUILD.md](BUILD.md) 的「发布前必须解决」。
- **远程模式的 AI 助手**：助手在客户端进程里跑，远程模式下它读的是 host 的数据（通过
  `RemoteBackend`），但布局仍存在本机 `ui.db`。多设备之间布局不同步。
- **申报期的扣除率要等年度结束才公布**：IRD 的公里费率和平方米费率在 income year
  结束后才发布，所以当年的规则文件必然带占位值，`rules/nz/2026-27.yaml` 里标了
  `needs_verification`。
- 对话记录不落盘。
