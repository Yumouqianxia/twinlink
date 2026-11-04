# 界面与实现委托记录

2026-11-04 这轮界面改造及主题运行时由 DeepSeek Harness（项目配置模型 `deepseek-v4.1-flash`）实际执行；Codex 负责任务约束、源码审阅、独立回归、版权声明及验收。调用凭据与 DSH 对话保留在项目外的私有目录，不随代码和安装包分发。

最终形态参考本机官方 Harness 0.2.1-rc.5 的 `ui-theme`、`ui-layout`、`ui-sidebar`、`ui-settings` 和 `ui-chat` 模块：浅灰侧栏、紧凑顶栏、圆角输入面板、统一线性图标、成组设置与一致的明暗语义色。未引入 Harness、Vue 或 Cordis 运行时，保留 vanilla TypeScript、Vite 及 TwinLink 自有双条标识。参考及适配范围的 MIT 版权声明保留在 `THIRD_PARTY_NOTICES.md` 和 `third-party/deepseek-harness/LICENSE`，并列入 Tauri 打包资源。

`src/themes.ts` 管理声明式 JSON 主题包与本地注册表：四套内置主题、用户主题安装／启用／替换／卸载、明暗及系统跟随、强调色和字号、热切换及本机持久化。外观控件在 `src/main.ts`，样式消费 `--tl-*` 语义变量；主题格式与示例见 [THEMES.md](THEMES.md)。

本轮协调允许 `src/updates.ts` 做当前运行版本的文本展示修正，仍由后端报告的版本及打包版本作为来源；没有改变 GitHub 更新源限制、签名验证、显式安装确认或 Rust/API 契约。其余更新流程、配对、分页消息、P2P／离线文件、拖拽、自动接收和数据备份通过现有 API 保留。

浏览器继续明确显示“界面预览”；产品代码没有演示回复、伪造对端、虚构在线状态或模拟传输。实际构建与测试、截图及平台限制见 [CLIENT-VALIDATION.md](CLIENT-VALIDATION.md)。
