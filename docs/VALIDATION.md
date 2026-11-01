# 初版验证记录

验证日期：2026-10-30，Windows x64。本文件记录已实际执行的项目检查，不将
本机回环测试当成真实异地 P2P 测试。

## 已通过

- 前端：TypeScript 严格模式检查、Vite 生产构建。
  输出 JS 约 20 KB、CSS 约 16 KB（未压缩），没有外部字体或图片。
- Rust core：8 项测试通过；另 1 项依赖官方引擎的测试默认忽略，已单独执行通过。
  覆盖双向消息、文件接收确认、拒收、重名不覆盖、源内容变化校验失败、空文件、
  文件名路径保护、错误密码及超大帧拒绝。
- Tauri：`cargo check -p twinlink` 通过；Windows x64 release 编译通过，
  启用 `custom-protocol`，界面嵌入可执行文件，不依赖 Vite 服务。
- 原生窗口：Windows WebView2 窗口实际打开，显示 `http://tauri.localhost/`；
  偏好设置成功显示 Rust 返回的 `C:\Users\Admin\Downloads\TwinLink` 下载目录。
- 浏览器交互：连接对话框、创建/加入角色与虚拟地址互换、原生功能在预览中的
  明确提示、文件传输空状态、下载目录设置入口。
- 官方 EasyTier 2.6.4：两个独立进程在同机、免 TUN 模式下组成私有网络，
  经本地端口转发成功双向回显数据。
- 应用实际 EasyTier 启动参数：独立动态管理端口、动态本地转发端口、
  no-tun/smoltcp、TCP 白名单、环境变量密码均被引擎接受；进程与转发监听正常。

## 打包时修正

- EasyTier 官方 Windows core 静态导入 Packet.dll。缺少它会在程序入口前退出，
  即使没有启用 TUN 也一样。安装脚本与便携包均已补齐运行库，未安装驱动。
- EasyTier 的自动管理端口在多实例同时启动时可能竞争；应用改为分配独立的
  loopback 端口。本地转发也使用动态端口以减少重连争用。

## 内存采样

这台机器的默认硬件渲染空闲采样：应用及其 WebView2 进程的私有提交合计约
209 MiB，工作集合计约 430 MiB。主 Rust 进程的私有提交约 6 MiB。
只看主进程会明显低估整体开销。

软件渲染采样：同一界面下，私有提交合计约 113 MiB，工作集合计约 370 MiB。
因此本原型的 Windows WebView2 配置使用 `--disable-gpu`；该界面没有视频、
3D 或持续动画。它会把部分渲染工作转给 CPU，需要在其他显卡和长聊天列表上
继续回归；不把单台机器的两次采样当作通用性能保证。
该参数含义见 [Microsoft WebView2 文档](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/webview-features-flags)。

原始采样在 `windows-idle-memory.json` 与
`windows-idle-memory-software-rendering.json`，包含应用及其 WebView2 后代进程。
工作集合计会重复计算共享页，私有提交也不等同于实际常驻物理内存。
上述采样处于断开连接状态，不包含 EasyTier。联机与大文件时还需测量。

## 未验证 / 未实现

- 未实际构建或运行 macOS；已提供项目配置和 Windows/macOS CI 检查工作流，
  工作流未推送运行。
- 未在两台真实电脑、不同运营商或公司防火墙环境下测试打洞成功率。
- 未做长时间运行、休眠恢复、超大文件、磁盘写满等故障注入。
- 当前重连为手动操作，文件取消后重新发送，没有断点续传。
- 消息成功发送表示加入本地发送队列，没有送达/已读确认协议。
- 传输记录已入库，但重启后的历史传输浏览尚未接入界面。
- Windows 签名、macOS 签名公证和正式安装器未完成。

## DeepSeek 协作

已安装项目级前端设计/界面测试技能，并实际启动 DSH headless。
使用现有桌面配置的 DeepSeek 账号路由后，服务返回 ACCOUNT_QUOTA / HTTP 402
Insufficient Balance。本次 UI 由 Codex 完成，原任务说明保留在 UI-BRIEF.md。
