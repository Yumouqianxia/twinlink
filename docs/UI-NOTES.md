# 界面与实现委托记录

当前界面和客户端工程由 DeepSeek Harness 参与实现，Codex 负责协调协议、
服务端部署、代码复核和独立验收。

DSH 使用本项目专用配置，通过自定义 OpenAI-compatible 接口调用
`deepseek-v4.1-flash`。凭据只保存在仓库外的私有配置目录；本文件不记录密钥。
早期 `deepseek-account` 路由曾返回 402，后来已使用用户指定接口完成有效调用，
不能再用那次失败描述当前实现。

项目级技能 `.agents/skills/frontend-design` 和 `webapp-testing` 可供 DSH 使用。
界面采用 vanilla TypeScript、CSS 和内联 SVG，不下载远程字体或图片。
中文界面包含会话、文件传输、配对和设置。浏览器预览明确标注，实际功能走
Tauri IPC；不提供虚构的在线或传输状态。

最初 UI 委托限定 `src/main.ts`、`src/styles.css`、`index.html`；用户随后明确授权
继续实现 Rust/API 与编译，工程阶段已协调解除该三文件限制。Rust core 与
协议改动仍以经过协调的服务契约为准。

实际编译、浏览器检查与原生窗口检查见 `CLIENT-VALIDATION.md`。早期黑屏截图
不能作为原生窗口成功的依据，最终验收应检查打包的 `.app` 和可见的原生控件。
