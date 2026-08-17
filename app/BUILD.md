# 编译与分发

日常开发和配置见 [README.md](README.md)。本文档讲**怎么产出可安装的包，以及发布前还缺什么**。

下面的数字和行为都是在这台机器上实跑出来的（macOS / aarch64，2026-08-17），不是估算。

---

## 一条命令

```bash
cd app
npm run tauri build
```

它做三件事：`npm run build`（tsc + vite 打前端）→ `cargo build --release`（编译 Rust，
连同 `service/` 的五个库 crate）→ 打包。

产物（macOS）：

```
src-tauri/target/release/financeapp                       14 MB   裸二进制
src-tauri/target/release/bundle/macos/Finance.app         15 MB   可直接双击
src-tauri/target/release/bundle/dmg/Finance_0.0.1_aarch64.dmg  5.7 MB   分发用
```

作为对照，同类 Electron 应用的安装包通常在 100 MB 以上。

### `.app` 里只有一个可执行文件

```
Finance.app/Contents/
├── Info.plist
├── MacOS/financeapp        ← 唯一的可执行文件
└── Resources/icon.icns
```

没有 `taxweb`、没有 `taxmcp`、没有 sidecar。报税引擎的五个 crate 已经**编译进了
`financeapp` 这一个二进制**，运行时直接打开 `ledger.db`。桌面版从头到尾只有一个进程。

---

## 前提

`../service/` 必须与 `app/` 同级存在——`src-tauri/Cargo.toml` 用相对路径依赖它的 crates。

这是**编译期**依赖。编译完成后二进制里已经包含那些库的代码，
分发出去的 `.app` 不需要 `service/` 目录，用户机器上也不需要 Rust 工具链。

---

## 各平台

Tauri **不做交叉编译**——每个目标平台都要在对应的系统上构建（或用 CI 矩阵）。

| 平台 | 产物 | 需要 |
|---|---|---|
| macOS | `.app`、`.dmg` | Xcode command line tools |
| Windows | `.msi`、`.exe`(NSIS) | MSVC 工具链、WebView2 Runtime |
| Linux | `.deb`、`.rpm`、`.AppImage` | `libwebkit2gtk-4.1-dev`、`libssl-dev` 等 |

`tauri.conf.json` 里 `bundle.targets` 是 `"all"`，即当前平台能出的都出。
只要某一种就改成数组，如 `["dmg"]`。

在 Apple Silicon 上默认只出 arm64。要 Intel 或通用二进制：

```bash
npm run tauri build -- --target x86_64-apple-darwin
npm run tauri build -- --target universal-apple-darwin
```

### 移动端

```bash
npm run tauri ios build
npm run tauri android build
```

iOS 需要 Apple 开发者账号和签名配置；Android 需要 keystore。
移动端没有本地账本，装机后必须在应用的「设置」里填 `host` 指向常驻 host。

---

## 🔴 发布前必须解决

**当前的构建产物能跑，但还不能发给别人用。** 下面几条都是实测出来的。

在一个干净的 `HOME` 里启动打包好的 `.app`：

```
financeapp: 数据来源 本地账本 <fresh>/.taxdata
financeapp: AI 助手未启用（未设置 API key）
financeapp: 前端已连接
```

它建了 `~/.taxdata/ledger.db`，**没有建 `~/.taxdata/rules`**。这就是第一个问题。

### 1. 规则文件没进 bundle（严重）

`rules/nz/*.yaml` 不在 `.app` 里——`Contents/Resources/` 只有一个图标。
`config.rs` 默认把规则目录解析成 `<data-dir>/rules`，全新安装时那个目录不存在。

后果：总览页和文档页正常，**GST 页和所得税页直接报「no rule file for NZ ...」**。

规则是数据不是代码，所以正确做法是把它们当资源打包：

1. `tauri.conf.json` 的 `bundle` 里加 `resources`，把 `../../service/rules` 映射进包；
2. `config.rs` 的解析顺序改成：`FINANCE_RULES_DIR` → 打包资源目录
   （`app.path().resource_dir()`）→ `<data-dir>/rules`。

保留环境变量优先，开发时仍然可以指向仓库里的 `rules/`。

> 具体的 `resources` 路径映射我没有实测过（这属于改代码，不属于写文档）。
> 要我直接做掉就说一声。

### 2. GUI 启动读不到 shell 的环境变量（已解决大半）

macOS 上从 Finder / Dock 启动的应用继承的是 launchd 的环境，**不是你 shell 的**。
所以 `FINANCE_DATA_DIR`、`FINANCE_RULES_DIR`、`ANTHROPIC_API_KEY` 全都读不到。

现在 API key 和远程 host 有了**设置界面**，存在应用数据目录的 `settings.json`
（权限 `0600`），不再依赖环境变量。环境变量仍然优先，只是打包后没人设得上。

剩下的：

| 变量 | 打包后的出路 |
|---|---|
| `ANTHROPIC_API_KEY` | ✅ 设置界面 |
| `FINANCE_HOST` | ✅ 设置界面（改完要重启） |
| `FINANCE_RULES_DIR` | ⬜ 靠上面第 1 条（打进 bundle）解决 |
| `FINANCE_DATA_DIR` | ⬜ 默认 `~/.taxdata` 够用；真要改还得加设置项 |

### 3. dev 的 CSP 混在生产配置里（轻微）

`tauri.conf.json` 的 `connect-src` 里带着 `ws://localhost:1421`（Vite HMR 用）。
生产构建用不到，留着也不会被利用（没有东西去连它），但不该出现在发布配置里。

修法是拆成两份 config，`tauri build` 时用不含 dev 来源的那一份。

---

## 签名与公证（macOS）

当前产物是 **ad-hoc 签名**：

```
CodeDirectory ... flags=0x20002(adhoc,linker-signed)
```

这只够在本机跑。别人下载后会被 Gatekeeper 拦下（「无法验证开发者」）。

要正常分发需要：

1. Apple Developer Program 账号（付费）
2. Developer ID Application 证书
3. 在 `tauri.conf.json` 配 `bundle.macOS.signingIdentity`
4. 公证（notarization）：构建时提供 `APPLE_ID`、`APPLE_PASSWORD`（app-specific）、
   `APPLE_TEAM_ID`，Tauri 会调用公证流程
5. Stapling（把公证票据钉进包里，让离线首次打开也能通过）

Windows 同理需要代码签名证书，否则 SmartScreen 会警告。

**自用不需要这些**——本机构建、本机运行就行。只有要发给别人时才必须。

---

## 版本号

`tauri.conf.json` 的 `version` 是包版本的权威来源（当前 `0.0.1`），
`src-tauri/Cargo.toml` 的 `version` 是 crate 版本。发布时两处一起改，别只改一处。

`.dmg` 文件名里带的就是 `tauri.conf.json` 的那个。

---

## 自动更新

**没有配置。** Tauri 有 updater 插件，需要：签名密钥对、一个放 manifest 的
静态地址、以及在 `tauri.conf.json` 里配 endpoint。

考虑到这个 app 读的是本地账本，更新推送要不要做取决于你打算给几台机器装。
自用的话手动换 `.app` 就够了。

---

## 发布前自检

```bash
cd app
npm test                       # 前端纯函数
npm run build                  # 前端类型检查
cd src-tauri && cargo test     # Rust
cd ../../service && cargo test --workspace   # 引擎
```

四项全绿之后再 `npm run tauri build`。

最后在**干净的 HOME** 里跑一次打包产物，确认全新安装可用：

```bash
FRESH=$(mktemp -d)
env -i HOME="$FRESH" PATH=/usr/bin:/bin \
  ./src-tauri/target/release/bundle/macos/Finance.app/Contents/MacOS/financeapp
```

这一步正是上面那几个问题被发现的方式——它跳过了你 shell 里所有的 `export`，
看到的东西和别人第一次装上时看到的一样。
