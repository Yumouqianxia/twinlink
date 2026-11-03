# 应用更新与 GitHub 发布

v0.0.2 是非正式预发布版，包含 Windows 下载目录异常时的启动修复。若旧版无法启动，请从 Release 页面下载完整安装包手动升级。更新覆盖完整的 Tauri 应用（前端、Rust 和独立引擎），需要重启；不是在运行中替换 Rust 或加载远程网页代码。

## 使用

应用启动时检查一次更新；也可在“设置 → 应用更新”手动检查。发现新版本后，由用户点击“更新至 v… 并重启”。下载失败或签名不匹配时不安装，原有应用与聊天记录保留。应用包与用户数据目录分开，正常更新不删除房间、历史、待发队列和传输记录。更新期间请保持应用开启。

公开下载不含联机服务器访问令牌。首次安装需导入自己的服务器设置码，或使用已有房间的有效邀请。GitHub 只承载代码、更新清单和安装包，不承载聊天或文件信箱。

## 固定更新源

- 清单：`https://raw.githubusercontent.com/Yumouqianxia/twinlink/updates/latest.json`
- 安装包：`https://github.com/Yumouqianxia/twinlink/releases/download/<版本标签>/<文件名>`
- 允许 GitHub 官方 Release 下载 CDN 的 HTTPS 重定向，不接受其他仓库、HTTP、任意外部域名或自定义更新源。
- 前端仅能请求“检查”和“安装”，未授予可自定义端点/公钥的 updater 插件 IPC 权限。
- 内置 Tauri updater 公钥验证每个包；启用 `requireSignedVersion`，签名必须绑定清单版本。禁止降级安装。
- 0.x 版本在 GitHub Release 中标为 prerelease。固定 `updates` 分支清单可发布预览版本，不依赖忽略 prerelease 的 `/releases/latest`。

该仓库的 `updates` 分支已完成首次初始化，只保存公开的 `latest.json`；后续 Actions 仅更新此文件，不把源码或工作流文件复制到更新分支。

本地开发预览不检查或安装更新。不能访问 GitHub 时显示错误并保留现有版本，不切换镜像或私人服务器。

## 发布下一版

1. 同步修改 `package.json`、`src-tauri/tauri.conf.json`、`src-tauri/Cargo.toml` 和 `crates/twinlink-core/Cargo.toml` 的版本，例如 `0.0.2`；更新 `Cargo.lock` 与 `docs/RELEASE-NOTES.md`。
2. 执行 `pnpm build`、`cargo test -p twinlink-core --locked`、`cargo test -p twinlink --locked`、`cargo check -p twinlink --locked`。
3. 提交并推送源码，再推送 `v0.0.2` 标签。`Release` 工作流为 macOS Apple Silicon、macOS Intel 和 Windows x64 分别构建。
4. 三个平台全部成功后，工作流创建草稿 Release、上传签名包与 SHA256SUMS、发布，然后才推进 `updates/latest.json`。

发布脚本验证四处版本一致、标签匹配、包哈希及来源，并拒绝覆盖已发布版本或把更新清单回退。失败的工作流不推进更新源。也可在 Actions 手动运行 Release，选择要发布的分支或 tag，并输入与源码版本匹配的版本标签；工作流固定检出触发时的提交，全部构建和 Windows 安装检查通过后才创建尚不存在的标签。既有标签必须指向同一源码提交，绝不自动移动。

签名私钥使用仓库 Actions Secret `TAURI_SIGNING_PRIVATE_KEY`，仅发布构建步骤可读；PR 检查不接触该密钥。本机私钥在项目目录之外，权限 0600；请独立备份，不要与模型 API 密钥一起销毁。丢失更新签名私钥会使已安装客户端无法验证后续新包。

macOS ad-hoc 签名与 Tauri 更新签名用途不同：前者让本地应用包结构可验证，后者认证发布者及更新包。当前没有 Apple 开发者签名/公证，也没有 Windows 商业代码签名；这不应被描述成已获得系统发行者认证。
