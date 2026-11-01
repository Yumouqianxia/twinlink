# TwinLink

Small Windows/macOS Tauri 2 application for two people: EasyTier networking,
real-time text chat and explicit-consent file transfers. Do not implement NAT
traversal yourself. EasyTier is an independently distributed executable.

Frontend: vanilla TypeScript + Vite; no heavy component library, CDN fonts,
analytics, remote assets, fabricated online states, or simulated real transfers.
Browser preview must clearly say it is a preview; Tauri IPC powers real features.
User-facing UI language is Simplified Chinese. Working product name: 同频 / TwinLink.

The Rust core belongs in crates/twinlink-core; src-tauri is a thin command/event
adapter. Large file data never crosses frontend IPC. Only explicitly selected
files can be sent. Receiving never overwrites existing files or follows a remote
path. Do not expose browsing of the peer's filesystem.

DeepSeek UI task may edit only src/main.ts, src/styles.css and index.html.
Do not change src/api.ts, Rust, protocol or dependency files without coordination.
Use local frontend-design and webapp-testing skills in .agents/skills as needed.

Validation: pnpm build; cargo test -p twinlink-core; cargo check -p twinlink.
Document actual validation and limitations, never imply cross-network or macOS
testing was performed if it was not.
