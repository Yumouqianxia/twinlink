# 同频 / TwinLink

当前版本：**v0.0.3 预发布版，非正式版**。

[下载 v0.0.3](https://github.com/Yumouqianxia/twinlink/releases/tag/v0.0.3) · [更新与发布说明](docs/UPDATES.md)

v0.0.3 加入可安装的主题包、数据备份、在线 P2P 文件传输、拖拽发送，以及聊天中的附件记录和图片预览。保留 Windows 启动修复与 macOS 图标边距修正；安装包与聊天数据分开保存。

两个人使用的 Windows/macOS 桌面聊天与文件传输应用原型。采用 Tauri 2、
TypeScript/Vite、Rust 和独立分发的 EasyTier 2.6.4。EasyTier 负责网络发现、
打洞与中继；项目不自行实现 NAT 穿透，也未复制 Astral 的源码或界面。

两端均更新到 v0.0.3 后可使用本轮新增功能。图片预览需本地文件仍存在，收到的图片须接收完成；备份只保存记录和配置，不包含附件本体。详见[附件说明](docs/ATTACHMENTS.md)与[主题包说明](docs/THEMES.md)。

## 配对和使用

1. 在“设置”导入管理员提供的服务器设置码；公开下载包不含私有服务器凭据。
   设置码包含访问凭据，请只通过可信渠道分享。
2. 一方选择“创建房间”，将生成的配对码发给另一方。另一方选择“输入配对码
   加入”，无需输入对方 IP、网络名或密码。
3. 同一配对码派生双方的私有网络身份和虚拟地址。保存的房间在应用重启后恢复；
   手动暂停后，只有点击恢复才重新联网。
4. 文件可选择或拖入发送；有直连时先发送邀请，同意后 P2P 流式传输，不预上传服务器，也不设应用文件大小限制。无直连时加密暂存服务器，单文件最多 1 GiB。接收默认询问，可在设置明确开启当前房间自动接收；更换房间后关闭。
5. 设置中可导出／导入聊天、传输记录和保存的房间／服务器凭据。备份未加密，不含附件本体；导入合并去重，保留本机目录，暂停连接且不重放发送队列。应用不会浏览对方磁盘，也不覆盖已有文件。

配对码包含房间密钥，等同于房间钥匙。任何持有码的人都可能加入，请勿公开。
目前仅支持两人房间，不提供多成员管理或设备公钥白名单。

## 外观与主题插件

侧栏“外观”（或设置中的外观入口）支持浅色、深色、跟随系统，内置静默蓝、林间绿、暖石墨与暮紫。可调整强调色和字号，选择会在本机保存。

主题是可导入、切换、导出和卸载的 JSON 插件包，安装和换肤无需重新编译。制作自己的主题可先导出一套主题，再修改名称、ID 与配色。格式及示例见 [主题插件说明](docs/THEMES.md)。

界面与主题架构参考 MIT 许可的 DeepSeek Harness，保留其版权声明；TwinLink 保持自己的名称与标识。

## 服务与保存周期

- 私有 EasyTier 服务协助发现和打洞；客户端实时通道强制仅 P2P，不经服务器中继文件。无法直连时使用独立信箱。
- 私有 HTTPS 信箱提供密文消息和附件暂存，与 EasyTier 服务独立。
- 服务器暂存内容保留 7 天，附件容量总计 4 GiB，单文件上限 1 GiB。
- 本机聊天记录保存到 SQLite，不因服务器过期而删除。数据库目前是本机明文，
  依赖操作系统用户目录权限保护，不提供数据库静态加密。
- 当前免 TUN 模式承载应用通信。真实虚拟网卡与局域网游戏模式尚未完成。

协议见 [SERVER-PROTOCOL.md](docs/SERVER-PROTOCOL.md)，部署与维护见
[SERVER-DEPLOYMENT.md](docs/SERVER-DEPLOYMENT.md)。客户端离线功能的实现和
实际验收范围以 [CLIENT-VALIDATION.md](docs/CLIENT-VALIDATION.md) 为准。

## 开发与构建

需要 Node.js 22.12+、pnpm 11+、Rust stable、Python 3；发布脚本需要 Python 3.11+。macOS 需要 Xcode
命令行工具；Windows 需要 Visual Studio C++ Build Tools、Windows SDK、WebView2。

```sh
pnpm install
python3 scripts/install_engine.py
pnpm tauri dev
```

引擎下载脚本从官方 Release 获取固定平台版本并校验 SHA-256。EasyTier 作为
独立进程启动，免 TUN 模式不安装网卡驱动。系统防火墙可能询问应用联网权限。

```sh
pnpm build
cargo test -p twinlink-core
cargo check -p twinlink
pnpm tauri build --bundles app --config '{"bundle":{"createUpdaterArtifacts":false}}'  # macOS 本机测试包
```

Windows 和 macOS 由 GitHub Actions 分别构建。macOS 使用本地 ad-hoc 签名，
未做 Apple 公证；Windows 未使用商业代码签名证书。跨设备实测仍待完成。
使用 Tauri CLI 打包会正确处理内嵌前端与资源；直接运行
未打包的 Cargo 二进制不作为 Mac 最终交付方式。

仅预览界面可运行 `pnpm dev`。浏览器中明确显示“界面预览”，原生配对、聊天、
文件选择与传输通过 Tauri IPC 执行，不模拟成功或在线状态。

## 项目说明

Rust 业务逻辑位于 `crates/twinlink-core`，`src-tauri` 仅作命令与事件适配。
大文件内容不经过前端 IPC。结构与本地文件位置见
[ARCHITECTURE.md](docs/ARCHITECTURE.md)。

项目已安装 DeepSeek Harness，并通过本次项目专用的自定义接口与模型调用，
用于 UI 和客户端实现；Codex 负责部署、审阅与独立验收。密钥不在项目中。
UI 委托及历史背景见 [UI-NOTES.md](docs/UI-NOTES.md)。
依赖与分发说明见 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)。
