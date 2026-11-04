# TwinLink UI handoff

User direction: adapt the DeepSeek Harness desktop workspace and theme extension
model into TwinLink, retaining TwinLink branding and the vanilla TS/Vite stack.
The reference is the installed MIT-licensed Harness 0.2.1-rc.5, especially
packages/client/ui-theme, ui-layout, ui-sidebar, ui-settings and ui-chat.
Keep attribution in THIRD_PARTY_NOTICES.md and third-party/deepseek-harness/LICENSE.

Coordinated UI-owned files: src/main.ts, src/styles.css, src/themes.ts, index.html.
Do not change IPC, backend, dependencies or capabilities without coordination.

Use semantic color tokens throughout light/dark/system appearances, including
native-looking dialogs, input, selected/disabled/focus/error states and transfers.
Theme plugins are versioned, declarative JSON packages with a local registry:
install, activate, export and uninstall, plus local appearance preferences.
Validate manifests and all values before installation; invalid input must not
change the active theme or installed plugins. No executable theme code, remote
assets, downloaded fonts, raw CSS or arbitrary HTML. Store only non-secret theme
preferences under a dedicated versioned localStorage key.

Preserve all existing working features and stable IDs/selectors: server import,
room pairing/pause/resume, message history and delivery receipts, P2P/offline file
consent and drag sending, auto-accept, backups and explicit GitHub updater flow.
Do not fabricate online states, peers, messages or transfers in product code.
Browser preview remains clearly marked. Do not serialize credentials to themes.

Review at 1160x780 and 880x620, light and dark, with scrollable settings and visible
keyboard focus. User-controlled text must be escaped; keep draft/focus across
incoming events. Honor reduced-motion preferences and the existing CSP.

Use .agents/skills/frontend-design and webapp-testing. Validate pnpm build and
existing file/update UI regressions. Keep real native/cross-network validation
claims separate from browser fixtures. Codex coordinates licensing, final
regression testing and publication; DSH should not commit, push or create tags.
