# 同频 / TwinLink

当前版本：**v0.0.2 预发布版，非正式版**。

[下载 v0.0.2](https://github.com/Yumouqianxia/twinlink/releases/tag/v0.0.2) · [更新与发布说明](docs/UPDATES.md)

v0.0.2 包含 Windows 下载目录异常时的启动修复、启动失败提示与本地日志，以及 macOS 图标修正。若 v0.0.1 已无法启动，请下载新版安装包手动升级；安装包与聊天数据分开保存。

两个人使用的 Windows/macOS 桌面聊天与文件传输应用原型。采用 Tauri 2、
TypeScript/Vite、Rust 和独立分发的 EasyTier 2.6.4。EasyTier 负责网络发现、
打洞与中继；项目不自行实现 NAT 穿透，也未复制 Astral 的源码或界面。

## 配对和使用

1. 在“设置”导入管理员提供的服务器设置码；公开下载包不含私有服务器凭据。
   设置码包含访问凭据，请只通过可信渠道分享。
2. 一方选择“创建房间”，将生成的配对码发给另一方。另一方选择“输入配对码
   加入”，无需输入对方 IP、网络名或密码。
3. 同一配对码派生双方的私有网络身份和虚拟地址。保存的房间在应用重启后恢复；
   手动暂停后，只有点击恢复才重新联网。
4. 文件由发送方主动选择，接收方明确同意后才写入下载目录；应用不会浏览对方
   的磁盘，也不会覆盖本机已有文件。

配对码包含房间密钥，等同于房间钥匙。任何持有码的人都可能加入，请勿公开。
目前仅支持两人房间，不提供多成员管理或设备公钥白名单。

## 服务与保存周期

- 私有 EasyTier 服务负责发现和中继；客户端由 EasyTier 自动选择可用路径。
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
