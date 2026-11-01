# DeepSeek UI handoff

Build the production-quality Chinese desktop UI on top of src/api.ts. You own
src/main.ts, src/styles.css and index.html only. Preserve the vanilla TypeScript
and Vite stack. The core/backend will be implemented separately in parallel.

Read .agents/skills/frontend-design/SKILL.md. Use a calm, distinctive desktop
workspace: warm paper background, forest-green accents, restrained typography,
subtle borders, excellent spacing. No purple gradients, dashboard KPI cards,
external fonts, stock illustrations, or online-looking demo data. The app is a
small focused desktop tool, not a marketing site. Minimum window 880x620.

Layout: narrow sidebar (brand 同频 / TwinLink, 工作会话, 文件传输, 设置), main
conversation area, peer connection status, optional compact file panel. Empty
state should be thoughtfully designed. Composer supports Enter to send and
Shift+Enter for newline. File transfer list supports incoming consent, decline /
cancel, progress, failures. No fake replies, peers, speed or progress values.

Connection modal: networkName, networkSecret (password input), bootstrap
(tcp://server:11010), role host/join. Defaults localIp/peerIp host 10.144.144.1/.2,
join swapped; allow editing under advanced settings. These virtual IP addresses
are fixed defaults for an initial 2-person room. UI labels explain room and
password match on both devices. Never persist networkSecret in localStorage.

Settings: display name; download directory chosen via api.chooseFolder.
Subscribe to api events before loading snapshot, merge by IDs. Use textContent
or proper escaping for user-controlled text, filenames and errors. Preserve
textarea focus/selection and ongoing dialog drafts while progress events arrive.
After send, insert the returned message by ID (events can arrive first).
File picker uses api.chooseFile -> api.offerFile. Incoming transfer buttons call
api.acceptFile or api.cancelTransfer. Show all backend errors in an accessible
toast/status. Disable actions when connection state makes them unavailable.

Browser mode is a visual preview only. api.snapshot supplies preview=true;
display a discreet but unambiguous preview label. Settings/connection forms may
be explored but actual native-only actions must explain that desktop is required.

Use inline SVG icons, CSS responsive layout, keyboard focus, dialog escape,
aria labels, reduced-motion support. Do not use emoji as primary app icons.
Run TypeScript/build checks using the supplied pnpm entry path if needed.
Write docs/UI-NOTES.md with changes and checks. Do not edit Rust or src/api.ts.
