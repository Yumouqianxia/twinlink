# 主题插件

在侧栏“外观”，或“设置 → 外观与主题”打开管理窗口。可使用四套内置主题，选择浅色、深色或跟随系统，并调整强调色与 12–18 px 字号。偏好及用户主题保存在独立的 `twinlink.appearance.v1` 本机存储中，不含聊天或服务器凭据。

## 制作、分享和安装

1. 选择接近目标的主题，点击“导出当前主题”。下载的小 JSON 也会显示在窗口中，可复制保存。
2. 修改 JSON 的 `id`、`name`、`version`、`light`、`dark` 和 `style`。内置 ID 不能覆盖；导出内置主题时会自动改为 `exported-…`。
3. 用“导入主题包文件”选择文件，或把 JSON 粘贴到文本框后点击“应用这份 JSON”。校验成功后即可使用，安装相同 ID 的主题需明确确认替换。
4. 删除当前用户主题会回退到默认主题；内置主题保留。“恢复默认外观”恢复默认主题及系统模式，不删除已安装主题。

主题格式为 `format: "twinlink-theme"`、`schemaVersion: 1`。名称和作者等为显示元数据；`id` 需为 3–48 位小写字母、数字、点、短横线或下划线，以字母或数字开头；版本形如 `1.0.0`。文件最多 64 KiB，最多安装 16 个用户主题。

`light` 与 `dark` 定义语义配色。可用颜色项为：`canvas`、`sidebar`、`surface`、`surfaceAlt`、`hover`、`active`、`ink`、`muted`、`faint`、`line`、`lineStrong`、`accent`、`accentHover`、`accentPress`、`accentSoft`、`accentLine`、`onAccent`、`danger`、`dangerSoft`、`warn`、`warnSoft`、`warnLine`、`ok`、`okSoft`。颜色值只接受 `#RRGGBB`；缺失项继承默认主题对应明暗模式的值。阴影和遮罩为内部材质，不属于可导入字段。

`style.radius` 为 0–18 的整数，`style.fontSize` 为 12–18 的整数。自定义强调色及当前字号会带入导出的主题包。非法颜色、未知字段、不支持的版本和原型污染字段会被拒绝；安装失败不会改变主题列表或当前主题。

可直接导入的完整示例：[example-warm-paper.json](../themes/example-warm-paper.json)。

## 扩展边界

本轮提供声明式主题插件及本地注册表。DeepSeek Harness 的 npm 插件不能直接装入 TwinLink，需把配色转换为上述 JSON 格式。功能面板、协议及其他业务插件还需要单独的扩展接口。

主题包不携带脚本、任意 CSS、远程字体或图片，也不获得文件或聊天访问接口。它们与聊天数据备份独立：迁移外观时另行导出／导入主题包。

参考实现为 [DeepSeek Harness](https://github.com/deepseek-ai/deepseek-harness) 的 `ui-theme`、`ui-layout`、`ui-sidebar`、`ui-settings` 与 `ui-chat` 模块。本项目采用语义 CSS 变量和独立的主题运行时，未引入 Harness/Vue/Cordis 运行环境。上游 MIT 声明保留在 [LICENSE](../third-party/deepseek-harness/LICENSE)，并随安装包资源分发。
