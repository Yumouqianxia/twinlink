# Third-party notices

- This application uses Tauri (MIT OR Apache-2.0), Vite (MIT), TypeScript
  (Apache-2.0), and Rust dependencies listed in Cargo.lock.
- Networking uses an unmodified, independently launched EasyTier v2.6.4 executable.
  EasyTier is LGPL-3.0. Its license is included at third-party/easytier/LICENSE.
  Corresponding upstream source: https://github.com/EasyTier/EasyTier/tree/v2.6.4
  Release: https://github.com/EasyTier/EasyTier/releases/tag/v2.6.4
  No EasyTier source has been linked into TwinLink's Rust crate.
- Astral (https://github.com/ldoubil/astral) was consulted to identify its
  EasyTier networking dependency. No Astral code, artwork, names, or UI was copied.
- Project-local design/testing skills were installed from
  https://github.com/anthropics/skills. Each skill retains its own included license.

Before distributing a release, generate a complete dependency license inventory,
include the relevant license texts, and provide the corresponding EasyTier source
in a manner satisfying its distribution terms. This prototype is not a completed
release compliance bundle.

- The UI theme tokens, workspace layout and appearance settings draw on
  DeepSeek Harness, locally installed version 0.2.1-rc.5 (MIT), copyright
  (c) 2026 DeepSeek. Upstream: https://github.com/deepseek-ai/deepseek-harness
  Relevant packages: packages/client/ui-theme, ui-layout, ui-sidebar,
  ui-settings and ui-chat. Adapted code and styles remain subject to that MIT
  notice, included at third-party/deepseek-harness/LICENSE. TwinLink does not
  include the DeepSeek whale logo, claim official affiliation, or embed the
  Harness runtime. Its own application source is licensed under LICENSE;
  bundled third-party components keep their respective licenses.
