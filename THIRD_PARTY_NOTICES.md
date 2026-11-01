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
