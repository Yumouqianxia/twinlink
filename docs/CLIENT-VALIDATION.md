# 客户端验证记录

## 2026-11-04 数据备份与 P2P 文件

从 GitHub `main` 的 `81af2ba` 拉取更新后实现，未更改服务器协议或容量，也未发布新 tag／安装包。

- 核心测试使用两套独立数据目录、真实 loopback TCP 认证会话和独立的内存信箱传输夹具。验证确认前接收目录不落盘、P2P 不创建服务器附件或请求服务器文件块、2 MiB+ 实际字节与哈希、同名不覆盖、双向 3/4 MiB 同时传输、拒收／取消、源文件同长度改变不发布、断线同 ID 回退、接收同意跨回退保留、空文件和离线自动接收。
- 使用实际 `1 GiB + 1 byte` 的稀疏普通文件走不限大小哈希与 P2P 邀请，验证大文件不再被信箱上限挡住，断线后不会转存服务器。没有声称已把超过 1 GiB 的文件跨网络完整传完。
- 备份测试覆盖超过当前 200 条历史分页的 205 条消息、空白设备恢复、现有房间与目录保留、重复导入不新增、不降级本机送达状态、不恢复未完成传输／发送队列、未来版本与无效记录拒绝、导出不覆盖。
- 原生 EasyTier 配置检查显式执行官方引擎 ignored 测试，确认 `--p2p-only true` 参数被接受并创建 loopback 转发监听。这个参数的语义见 [EasyTier 官方说明](https://easytier.cn/guide/network/p2p-optimize.html)；没有自己实现打洞。
- `scripts/test_file_ui.py` 用 Playwright 在 1160×780 下检查拖拽反馈、多文件去重、打开对话框时不发送拖入文件、自动接收开关、备份二次确认与导入结果；此部分明确使用 IPC／原生拖拽事件夹具。另以真实浏览器预览检查预览提示及拖拽拒绝，没有把夹具结果当作原生系统拖拽验收。

本机 `pnpm build`、`cargo test -p twinlink-core --locked`（65 passed、0 failed、1 ignored）、`cargo test -p twinlink --locked`（4 passed）及 `cargo check -p twinlink --locked` 通过；另显式执行官方引擎配置测试（1 passed）。双向旧版本握手兼容回归也已包含并通过。源码提交 `d86f380` 的 [Windows/macOS Check](https://github.com/Yumouqianxia/twinlink/actions/runs/41792847000) 两个平台均通过前端构建、核心／适配层测试及 Rust 检查。

本轮没有进行两台不同电脑的真实 NAT 打洞、Windows/macOS 原生拖拽、Windows 安装包或跨运营商传输验收。无法打洞但双方可访问信箱时仍使用离线信箱；大文件需要建立直连后发送。原生系统拖拽和跨机直连仍需双端实测。


## 2026-11-03 Windows 11 实机安装复核与 v0.0.2 发布准备

从公开 `main` 提交 `17f5a24` 拉取源码，在 Windows 11 专业版 x64（10.0.26200、WebView2 156.0.4314.58）复核。使用本机已有 Rust 1.99.1 GNU/MinGW 工具链，显式设置 `-C link-self-contained=yes`；Node 24.22.1、pnpm 11.25.0。首次缺少链接参数导致的编译失败属于本机工具链配置问题，补齐参数后构建通过。

实际完成 `pnpm install --frozen-lockfile`、`pnpm build`、`cargo test -p twinlink-core --locked`（50 passed、1 ignored）、`cargo test -p twinlink --locked`（4 passed）及 `cargo check -p twinlink --locked`。另显式执行官方 EasyTier 参数测试（1 passed），引擎采用官方 2.6.4 Windows x64 包，归档 SHA-256 为 `27af91e270e554709b048bd32327fefd2dfce5062ae1e8701af7550c6f525f84`。

通过 Tauri CLI 构建 NSIS 包，实际安装到当前用户的 Programs/TwinLink，安装器退出码 0。包中包含 EasyTier、Packet.dll 和 GNU 构建需要的 WebView2Loader.dll。本地测试包只关闭更新签名产物生成，发布包仍由 Actions 使用既有私钥签名。安装前备份已有应用数据；原生界面与设置页正常，显示 v0.0.2 和下载目录，日志出现 `[core-ready]`、`[ready]`、`[frontend-ready]`，确认实际前端已请求 Tauri IPC。正常退出后再次启动，日志再次记录前端就绪，进程持续运行至少 20 秒。

本机未修改用户的 Downloads 注册表位置或损坏用户数据库。异常下载目录与损坏数据库的整包故障注入由发布工作流在一次性 Windows runner 执行，三项检查全部通过才允许发布。此前撤回候选 tag 的记录属于历史；本次按新的发布请求准备 v0.0.2。此次未验证两台不同网络电脑的聊天或文件传输，也未在本机运行 macOS。

## 2026-11-03 Windows 启动问题调查

对公开 v0.0.1 NSIS 安装包做了实际安装和进程启动测试。GitHub 托管 Windows 2022（NT 10.0.20348，WebView2 131）与 Windows 2025（NT 10.0.26100，WebView2 156）在默认环境下均持续运行 20 秒；这项旧版测试仅判断进程存活，不等同于 GUI 功能全部通过。

在同样的两套隔离环境中，把 Windows 系统 Downloads 已知目录重定向到一个不可用路径，旧版均以 101 退出，stderr 为 `Failed to setup app: error encountered during setup hook: unknown path`。因此确认了一条真实闪退路径：启动回调强制依赖系统下载目录，Tauri 将其解析错误转成主线程 panic。此复现条件不能证明某位用户电脑的闪退一定由相同原因引起。

v0.0.2 修正为下载目录解析或创建失败时使用应用数据目录下的 `received`，不改变既有已保存的下载偏好；其他初始化错误明确显示提示并写本地启动日志，避免无提示退出。日志只写启动阶段、组件版本和初始化错误，不序列化聊天、房间或服务器配置。Windows 默认日志位置为 `%LOCALAPPDATA%\TwinLink\logs\startup.log`，不可写时回退到 `%TEMP%\TwinLink\logs\startup.log`。数据库初始化失败不清空或替换用户数据库。

发布流程增加“安装本次构建的 NSIS 包 → 正常启动 → 异常 Downloads 启动 → 损坏数据库受控退出”检查；正常/回退场景还要求实际前端通过 Tauri IPC 请求快照，不能仅靠进程存活判断成功。测试夹具只允许运行在一次性 GitHub Actions runner，目录重定向在结束时恢复。按用户要求，本次仅推送源码供 Windows 实机测试，不创建新 tag 或 Release；候选版的这些安装包检查尚未执行，不作为已通过项目。

候选版在本机通过 `pnpm build`、`cargo test -p twinlink-core --locked`（51 passed、0 failed、1 ignored）、`cargo test -p twinlink --locked`（4 passed）及 `cargo check -p twinlink --locked`。现有 GitHub 更新元数据仍指向 v0.0.1。

源码提交 `5f3d70c` 的 [Check 工作流](https://github.com/Yumouqianxia/twinlink/actions/runs/41655044992) 首次运行中，macOS 两项信箱文件测试超时，Windows 作业随矩阵失败取消；未改源码重跑后，Windows/macOS 的前端构建、核心和适配层测试、Rust 检查均通过。这不等同于候选安装包的 Windows 实机启动验收。先前触发的 v0.0.2 发布任务已取消，临时 tag 已撤回，没有新 Release。

## 2026-11-03 macOS 图标尺寸修正

只调整 macOS ICNS 的透明留白。以 1024 px 图层的 alpha ≥128 边界测量，原图可见宽度为 100%，本机 QQ 图标约为 81.05%；修正后的 TwinLink 为 81.25%（四边各留 96 px）。Windows 共用 PNG/ICO 未修改。后续通过 `node scripts/generate-macos-icon.mjs` 从共享 SVG 生成 macOS 图标，避免通用图标生成重新引入满幅底板。

本机已备份旧应用、替换 ICNS、重新进行 ad-hoc 签名并刷新系统图标登记；`codesign --verify --deep --strict` 通过，原生应用重新启动成功。`pnpm build`、`cargo test -p twinlink-core --locked`（51 passed、0 failed、1 ignored）、`cargo check -p twinlink --locked` 均通过。启动台的系统窗口未能通过自动化工具读取，因此不把图标边界测量描述成启动台截图对比验收。本次未发布新的 GitHub Release，也未改写既有 v0.0.1 下载包。

最终收尾日期：2026-11-01。本记录区分模拟界面回归、同机双进程和真实服务器验收；不代表两台不同网络的电脑已经联调。

## 环境和构建

macOS 15.1 arm64、Xcode 16.2、Rust/Cargo 1.99.1、Node 24.22.2、pnpm 11.28.10，独立分发的官方 EasyTier 2.6.4。

Rust 已持久安装到 `~/.cargo` 与 `~/.rustup`；DSH 已安装，项目入口为 `~/.local/bin/dsh-twinlink`，会话保存在 `~/.local/share/twinlink-dsh/sessions`。API 密钥在项目外的私有配置中，不随代码或应用分发。

最终源码、检查和 Release 构建均从 `/Users/jinchen/Desktop/twinlink` 执行。为复用缓存，使用 `CARGO_HOME=/tmp/twinlink-toolchain/cargo`、`CARGO_TARGET_DIR=/tmp/twinlink-build/target`；运行应用使用真实用户 HOME，无需重定向。

| 检查 | 实际结果 |
| --- | --- |
| `pnpm build` | TypeScript 与 Vite 通过；CSS 25.30 kB、JS 48.07 kB |
| `cargo test -p twinlink-core` | 51 passed、0 failed、1 ignored；无编译警告 |
| `cargo check -p twinlink` | 通过 |
| `cargo test -p twinlink` | 更新源和重定向限制的 2 项测试通过 |
| `pnpm tauri build --bundles app --ci` | 带更新签名的 Release `.app` 打包成功，约 30.17 MiB |
| 服务端 Python 单元测试 | 6 项通过 |

最终应用 v0.0.1 安装在 `/Users/jinchen/Applications/TwinLink.app`，已在真实 HOME 下打开原生窗口，配对入口可用，保留已导入的服务器配置，不预先创建用户房间。应用内包含独立 EasyTier 引擎。最终安装包使用 ad-hoc 签名，`codesign --verify --deep --strict` 通过。未做 Apple 开发者签名和公证；Windows x64 安装包现已通过云端构建和更新签名验证，但未做 Windows 真机安装验收。

## 核心和存储验证

核心测试覆盖配对与服务器设置码校验、密钥派生、快照脱敏、房间隔离、自动重连、手动暂停、原子状态落盘、消息去重及回执、显式接收与拒收、同名不覆盖、空文件、取消、错误密文隔离、上传恢复和容量错误。

额外覆盖 SQLite v1/v2/v3 重开、部分迁移恢复、未来版本拒绝、事务失败回滚，以及 451 条同时间戳消息分页无重复和跨房间游标拒绝。消息与加密发件箱在同一事务提交，投递状态不回退。旧记录无回执证据时显示“未确认”。

独立 PyNaCl 向量核对 Rust 的 XChaCha20-Poly1305 事件和分块密文。官方 EasyTier 精确启动参数的 ignored 测试此前已显式执行并通过；普通 `cargo test` 不运行该项。

## 真实海外服务器验收

服务器部署了 EasyTier 引导/中继与 HTTPS 加密离线信箱。验收使用两个独立 Core 数据目录、实际 Rust HTTP/加密实现和生成的测试文件，不使用模拟传输。

- 对端离线时发送，服务器暂存后仍显示未送达；发送方退出恢复不丢待发消息；对端上线后去重收取并返回持久化回执。
- 9 MiB 附件部分上传后暂停、退出，修改原始源文件，再恢复时仍使用此前加密缓存的原始字节；服务端已提交分块被复用。
- 接收方同意前不生成下载文件；部分下载后暂停、退出、恢复，最终逐字节一致；已有同名文件保留，接收文件自动改名。
- 双端完成回执、零字节附件、拒收、聊天和传输记录重启恢复均通过。
- 实际 Rust TLS 客户端接受正确私有 CA，拒绝错误 CA 和主机名。服务证书使用独立 CA:FALSE 叶证书，未关闭证书校验。
- 两个本机 EasyTier 进程在禁用普通 P2P 的实验中通过海外中继完成 TCP 回显；非授权命名空间被拒绝。

这些实验的两个客户端均位于同一台 Mac；海外服务器链路确实被使用，但不能据此推断两台独立网络设备的打洞成功率。

## 原生窗口验收

使用单独 bundle identifier 和数据目录的验收副本，以实际 Tauri IPC、实际 Core、实际海外服务器以及一个本机自动测试伙伴执行：

1. 伙伴未启动时，在原生文本框粘贴并发送，界面从“本机待发”变为“服务器已存”。
2. 启动伙伴后，原消息更新为“对方已收到”，无重复消息；伙伴的真实回复出现在会话中。
3. 收到 9.6 KB 测试附件，接收前本地目录为空；点击“接收”后显示“已完成”。原有同名文件内容保持不变，新文件字节与源文件一致。
4. 关闭验收副本和伙伴，安装并打开最终 Release 应用，核对中文界面和首次配对入口。

验收副本、生成文件和服务端记录只用于测试，收尾时清理，不混入正式用户房间或聊天历史。

## 浏览器界面回归

DSH 通过 Playwright 在 Chromium 中执行六组场景，均通过。此部分注入模拟 Tauri IPC，验证的是 UI，不代替上述真实网络和原生检查。

覆盖浏览器明确标记“预览”并拦截原生功能；未配对、房间已保存、暂停、重连、缺少引擎和信箱故障；离线仍可输入；投递状态原位更新并保留滚动位置；文件接收/拒收/取消；忘记房间二次确认并保留历史；200 条历史窗口加载更早记录、去重及较新回执不回退。无 pageerror，仅忽略浏览器自身 favicon 404。

## 交付语义与限制

- 本地聊天历史保存在 SQLite，重启后可读；可分页读取更早记录。卸载或删除应用数据仍会删除本地记录，当前没有云端永久备份。
- 服务端消息和附件加密暂存最多 7 天；`stored` 是服务器接受密文，`delivered` 是对端持久化确认，均不表示用户已读。
- 当前全部文件走 HTTPS 离线信箱，最大 1 GiB，接收方必须明确同意；在线文本另有 EasyTier 实时通道和信箱兜底。
- EasyTier 自动尝试直连并可回退中继，但未接入实际路由查询，界面不宣称已建立 P2P 直连。
- 未做两台不同电脑、不同运营商、Windows、游戏联机/TUN、长时间休眠恢复、1 GiB 边界或磁盘写满的实际验收。当前版本不提供游戏局域网模式，也未安装 TUN 驱动。
- 服务器服务证书有效至 2027-10-31 UTC，需到期前维护；维护脚本已放在服务器 `/opt/twinlink/deploy-server.sh`，私有 CA 到期前还需重新配置客户端信任。

凭据、房间种子与服务器设置码保留在项目外的私有目录；不得加入源码、截图、公开日志或分发包。

## v0.0.1 GitHub 更新验收

公开仓库为 `Yumouqianxia/twinlink`，`v0.0.1` Release 的 `isPrerelease=true`、`isDraft=false`。macOS Apple Silicon、macOS Intel、Windows x64 的构建、各平台核心/适配层测试和产物上传均通过；普通 macOS/Windows Check 工作流也通过。CI 曾遇到 Windows 默认文本编码和匿名 GitHub API 配额问题，最终代码显式使用 UTF-8，并在下载元数据时使用只读工作流认证。

固定清单 `https://raw.githubusercontent.com/Yumouqianxia/twinlink/updates/latest.json` 已匿名读取成功，含三个平台及其签名包地址。`updates` 是只含清单的独立分支，已完成首次初始化，并验证 Actions 后续更新清单成功。

原生完整流程使用实际 bundle 版本 0.0.0 的验收副本执行，没有模拟更新 API：点击“检查更新”发现 v0.0.1，再点击“更新至 v0.0.1 并重启”，从公开 GitHub 下载、校验签名、安装并自动重启。随后 bundle 版本变成 0.0.1，原生设置显示“当前已是最新预览版。更新源：GitHub。”。最终应用目录也已替换为该验证过的版本并重新启动。

此次更新前后的配置文件 SHA-256、SQLite 各表行数一致，未改动既有配置或数据；验收时正式数据目录没有聊天记录，因此不把这项对比描述成“带大量历史的整包更新实测”。大量历史分页、真实聊天/附件重启恢复已由前述 Core 验收覆盖。

另用与 updater 相同的 Minisign 验证库检查实际 macOS Apple Silicon 包与 Windows 安装包：签名及绑定的 0.0.1 版本有效，篡改包字节或签名中的版本注释都会被拒绝。Playwright 新增更新界面回归覆盖预览禁用、已最新、发现版本、纯文本发布说明、显式安装、失败后可重试，无 pageerror。

公开前对暂存源码、最终应用包做了实际 API 密钥、服务器访问令牌、更新签名私钥的值扫描，未发现泄露；签名私钥只保存在本机私有目录及仓库 Actions Secret。未做 Windows 或 Intel Mac 上的真实 GUI 安装/重启验收，也未做断电中的应用包替换实验。
