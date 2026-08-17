# Finance App —— 架构

> 客户端的技术选型、分层和不可动摇的约束。
> 服务端的愿景与阶段规划见 `../service/ROADMAP.md`，本文档只讲 app。
> 创建于 2026-08-17。重大方向调整时更新本文档，而不是另开新文档。

## 目标

1. **一套代码，桌面 + 移动**。macOS / Windows / Linux / iOS / Android。
2. **UI 由用户在运行时定义**。拖拽重排布局、增删组件、改文案、调样式，不需要重新编译、不需要发版。

第 2 条是这个 app 的形状决定因素，也是选型的主要约束。

## 选型：Tauri v2 + React + TypeScript

决定性理由是**后端已经是 Rust**。`service/` 里的 `taxcore` / `taxstore` / `taxingest` / `taxreturn` 都是库，
`taxweb` 和 `taxmcp` 只是套在库上的薄二进制。Tauri 的 core 本身就是一个 Rust 二进制，
所以 app 只是这些库的**第三个消费者**——直接 link，`#[tauri::command]` 直接调，
不走 HTTP、不开端口、不起独立进程。安全姿态比 `taxweb` 绑 127.0.0.1 更严一格。

被认真评估过的替代方案，以及淘汰原因：

| 方案 | 淘汰原因 |
|---|---|
| **Flutter** | 移动端最好，但 AOT 编译，UI 结构写死在二进制里。运行时可编辑需要引入 RFW/Stac 这类解释器，等于在 Flutter 里重造一个 DOM。金融图表生态也远弱于 web。 |
| **React Native / Expo** | 移动端强，桌面弱（RN macOS/Windows 维护滞后，Linux 无）。接 Rust 要走 FFI/JSI。 |
| **GPUI**（Zed） | Rust 原生、集成最紧，但**官方只支持桌面**（macOS/Linux/Windows），移动端只有第三方 early-development 的 `gpui-mobile`。且 pre-1.0、文档不全、widget 层薄，可编辑布局要从命中测试开始手写。第 1、2 条要求都不满足。 |

让出去的只有渲染层：系统 webview 而非 GPU 直绘。对一个看账本、审批分录、出报表的 app，
这个代价接近于零；换回来的是移动端、图表生态（ECharts / TradingView Lightweight Charts / TanStack Table）、
以及成熟的拖拽编辑生态（dnd-kit）。

## 分层

```
┌─────────────────────────────────────────────┐
│  React + TypeScript (src/)                  │
│    core/     LayoutDoc 类型、注册表、渲染器  │
│    blocks/   具体块实现（数据块 / 文本块）    │
│    editor/   编辑模式：拖拽、属性面板         │
└──────────────────┬──────────────────────────┘
                   │  invoke("...")  —— UI 永远只认这一个接口
┌──────────────────▼──────────────────────────┐
│  Tauri Rust shell (src-tauri/)              │
│    commands.rs   薄，只做 参数 → backend → JSON │
│    backend/      Backend trait               │
│      ├─ local    直接 link service 的 crates  │
│      └─ remote   HTTP 打到常驻 host 的 taxweb │
│    layout.rs     布局文档持久化（独立 db）     │
└──────────────────┬──────────────────────────┘
                   │
      ┌────────────┴────────────┐
      │                         │
┌─────▼──────┐          ┌───────▼─────────┐
│ 桌面：本地  │          │ 移动：瘦客户端    │
│ ledger.db  │          │ 经 Tailscale 连回 │
│ 同进程直读  │          │ host 的 taxweb    │
└────────────┘          └─────────────────┘
```

### 为什么移动端是瘦客户端

不做完整本地副本 + 双向同步。把 7 年的 append-only 审计账本复制到手机，
既是同步冲突地狱（对金融记录尤其不可接受），也是无谓扩大的风险面。

移动端通过私有网络（Tailscale / WireGuard）连回那台常驻 host。
**单一账本、无同步、provenance 链不分叉。** 手机仍然拿到原生 push 和生物识别锁。

这个差异被 `Backend` trait 完全吸收：UI 侧永远只调 `invoke("overview")`，
Rust 侧根据启动配置决定走 `LocalBackend` 还是 `RemoteBackend`。

## 交互方式：对话，不是设置面板

用户改界面的方式是**跟 agent 说话**，不是在属性面板里点选。
「把 GST 那三个数字挪到最上面」比找到三个块、各拖一次、各调一次宽度快得多，
而且不需要用户先理解块、栅格、绑定这些概念。

这条路径能成立，是因为下面那套 UI-as-Data 架构本来就是为它准备的：
布局是一份 JSON，改布局 = 产出一份新 JSON。agent 不需要任何特殊通道。

手动编辑（拖拽 + 属性面板）留着，但退到次要位置。它现在的主要价值是
**agent 那条路径的底层机制**——同一份布局文档、同一个撤销栈、同一套校验。
两条路径共用一切，所以不会出现「agent 能做但手动做不到」或者反过来的分裂。

### agent 的边界

```
┌─ webview ────────────────┐
│  聊天面板                 │  只发一句话、渲染事件
│  ✗ 看不到 API key         │
└───────────┬──────────────┘
            │ invoke / agent://* 事件
┌───────────▼──────────────────────────────────┐
│  Rust                                        │
│    agent/session  系统提示、对话历史、工具循环   │
│    agent/api      Claude Messages API（流式）  │
│    agent/tools    工具面                       │
│    agent/catalog  块白名单 ← 前端注册表推过来    │
└──────────────────────────────────────────────┘
```

**模型调用只发生在 Rust 侧。** API key 不进 webview——页面里任何东西
（包括 agent 自己生成的布局）都读不到它。

**工具表靠「没有」保证安全**，和 `taxmcp` 同一姿态：

| 有 | 没有 |
|---|---|
| 读账本（overview / GST / IR3 / 文档） | `approve_entry` / `reject_entry` |
| 读布局、查可用块型 | `ingest_document` |
| `apply_layout`（改呈现） | `set_document_status` |

唯一的写操作 `apply_layout` 改的是**呈现**，不是账；而且落库前必过块白名单校验。

**白名单来自前端注册表，不在 Rust 里写死。** 注册表是渲染器、属性面板和 agent
共用的同一份真相；在 Rust 里再抄一份迟早会漂移，而漂移的结果是 agent 生成出
渲染器认不出的块。前端启动时 `set_block_catalog` 把它推过去；目录没到位之前，
Rust 拒绝一切布局写入——**没有白名单就等于没有约束，那种情况下「通过」是错的**。

**agent 的每一次改动都可撤销。** Rust 改完 `ui.db` 后发 `agent://layout-changed`，
前端拉回新文档并**走 undo 栈**，用户一个「撤销」就能退回去。

## 核心：UI as Data

「用户可自定义 UI」不是一个功能，是渲染架构。核心是**布局文档化**——
界面不是组件树，是一份 JSON；渲染器读它，编辑器改它，agent 也改它。

```ts
type LayoutDoc = {
  version: 1                      // 文档结构版本，用于迁移；不是保存次数
  pages: Page[]
}

type Block = {
  id: string
  type: string                    // 只能取自组件注册表白名单
  layout: {                       // 同一份文档，两套断点
    desktop: { span: number }     // 12 栅格
    mobile: { span: number }      // 4 栅格
  }
  props: Record<string, unknown>  // 经该块 zod schema 校验
  copy?: Record<string, string>   // 文案覆写
  binding?: Binding               // 数据块的数字从哪来
}
```

页面还带一层**参数**：

```ts
type Page = {
  id: string
  title: string
  params: ParamDef[]              // 如 GST 页的申报期、所得税页的税年
  blocks: Block[]
}

type Binding = {
  source: string                  // 只能取自 ipc.ts 的 DATA_SOURCES 白名单
  path: string                    // "lines[code=gst101.box15].amount"
  agg: "value" | "count"
  params?: Record<string, string> // "$date" 引用页面参数
}
```

一个 GST 页面上的六个块看的是同一个申报期，所以「看哪一期」属于页面，不属于每个块。
参数的**定义**在布局文档里（用户可以加页面、加参数），**取值**不在——
那是会话状态，选一次日期就写一版布局是荒谬的。

页面参数同时是**块之间唯一的通信方式**。文档列表点选一行时写
`setParam("document", id)`，详情块的 binding 里写 `$document`——
两个块互相不知道对方存在，中间人是布局文档。这条规矩让主从联动这种典型需求
不需要在渲染架构之外再造一套跨组件状态，也让用户可以把详情块搬到别的页面、
或者放两个列表块各写各的参数，全都不用改代码。
不该给人看的参数（比如一个 UUID）标 `control: "hidden"`，参数条会跳过它。

`path` 支持 `[code=...]` 选择器而不只是下标：申报表的行以 `code` 标识，
用 `lines.10.amount` 绑定会在引擎调整行序时**静默指向另一个数字**。

块在页面上的**顺序就是数组顺序**，宽度就是 `span`。拖拽重排 = 交换数组元素，
调整宽度 = 改一个整数。刻意不用自由的 `{x, y, w, h}` 画布：
栅格模型已经同时覆盖「重排」和「缩放」两种需求，而且移动端断点是自然落下来的
（同一个块，两个 span），自由画布则要为每个断点各存一套坐标。
真需要自由画布时，`layout` 里再加一支即可，文档其余部分不动。

**组件注册表**是唯一的扩展点：`type` → React 组件 + props 的 zod schema + 编辑器面板定义。
渲染器和编辑器共用这一份定义，所以新增一个块型只需要注册一次，
渲染、属性面板、校验、序列化全部自动获得。

**状态与撤销**走 Immer 的 `produceWithPatches`：
白送 undo/redo，而且 patch 格式正好就是未来 agent 改布局的接口格式
（"帮我把 GST 那块挪到顶部" → LLM 产出 JSON patch → 同一条路径落库）。

## 不可动摇的约束

这三条是 `ROADMAP.md` 里架构原则在客户端的延伸，实现时不许绕开：

### 1. 布局是数据，不是代码

对应服务端的"规则是数据，不是代码"。

绝不允许用户注入 JS、表达式或任意 HTML。块型只能来自注册表白名单，
props 必须过 zod schema。渲染器遇到未注册的 `type` 显示占位错误块，
而不是尝试执行任何东西。

### 2. 可定制的是呈现，不是数字

对应服务端的"模型报告数字，从不计算数字"。

任何数字必须来自 `binding` 指向的 ledger 查询。用户可以改标题、改位置、改颜色、改单位显示，
**但不能手打一个数字进去**。否则用户（或 agent）能从 UI 侧绕开整条 provenance 链，
做出一个看起来权威、实际是编的数字。

因此块分四类（`BlockKind`），视觉上必须可区分：

| kind | 含义 | 视觉约定 |
|---|---|---|
| `data` | 值来自 binding，不可编辑 | 底部标注数据来源 |
| `text` | 自由文案 | 带「注」标记、虚线边框，明确是注释 |
| `action` | 会写入，但只造 pending（如收文档） | 绿色边框，与只读展示分开 |
| `gate` | 人工确认闸口 | 强调边框 + 「人工闸口」徽章 |

`action` 和 `gate` 的区别是**谁在决定**：`action` 把东西放进队列，`gate` 是人拍板。
收一份文档只产生 `PendingExtraction`，离账本还隔着提取、校验、生成草稿和 `gate` 四步。

### 3. agent 说数字，不算数字

对应服务端的「模型报告数字，从不计算数字」，同一条原则在 agent 层的落点。

agent 引用的每一个金额、税额、笔数，都必须来自工具返回的结果。它**不做算术**——
系统提示里明确写了这一条，工具面也不给它任何计算入口。用户问「这个数哪来的」时，
它用引擎返回的 provenance 回答，而不是复述自己的推理。

这条比前两条更难机械保证（模型总能在文字里写一个数），所以做了两层：
系统提示明确禁止，加上界面上数据块的值只能来自 binding——
agent 想把一个编出来的数字**放进界面**是做不到的。

### 4. 确认闸口是锁定块型

`approve` / `reject` 是整个系统里唯一的人工闸口（MCP 层刻意没有这两个工具）。
对应的块型标记为 `locked`：不可删除、不可隐藏、文案不可改写成有误导性的内容。
布局自定义不能削弱这个闸口。

## 已知短板

选型时就知道，不影响结论，但阶段三之前要解决：

- **移动端远程推送**：Tauri 官方只有 local notification，remote push（APNs/FCM）靠社区插件，可能要写原生胶水。
- **移动端跑不了常驻监控**：iOS 后台限制。阶段三的 cron agent 本来就该在 host 上跑，手机只做接收端——不是新增约束。

实现过程中冒出来的，需要服务端配合：

- **申报期的扣除率要等年度结束才公布**。IRD 的公里费率和平方米费率是在income year
  结束之后才发布的，所以当年的规则文件必然带着上一年的占位值。
  `rules/nz/2026-27.yaml` 里这两项标了 `needs_verification` 和明确的 note。
  app 目前不用它们（IR3 还没做法定扣除调整），但接入之前必须先处理这个状态——
  一个「暂用去年数字」的费率不能和一个确定的费率长得一样。

## 目录

```
app/
  ARCHITECTURE.md      本文档
  src/                 React 前端
    core/              LayoutDoc 类型、组件注册表、渲染器、状态、IPC 封装
    blocks/            具体块实现
    editor/            编辑模式
  src-tauri/           Rust shell
    src/
      lib.rs           启动、配置解析、命令注册
      config.rs        data_dir / rules_dir / 本地还是远程
      backend/         Backend trait + local + remote
      commands.rs      #[tauri::command] 薄封装
      layout.rs        布局文档持久化
```

布局文档存在**独立于 ledger.db 的 `ui.db`** 里，按版本追加。
账本是受审计的不可变记录，UI 偏好不是——两者不能共用一个库文件。
