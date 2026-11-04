import './styles.css';
import { initializeUpdates } from './updates';
import { api, type AppEvent, type Message, type Snapshot, type Transfer } from './api';
import {
  exportThemeJson,
  getAppearance,
  getTheme,
  initTheme,
  installTheme,
  listThemes,
  MAX_THEME_BYTES,
  paletteFor,
  resetAppearance,
  setAccent,
  setFontSize,
  setMode,
  setTheme,
  subscribeTheme,
  uninstallTheme,
  type ThemeMode,
  type ThemeSummary,
} from './themes';

/* Paint the stored appearance before the shell exists, so the first frame is
 * already in the right colour scheme and no white flash appears in dark mode. */
initTheme();

/* ---------- helpers ---------- */

const paths: Record<string, string> = {
  chat: '<path d="M20 15.2a2.8 2.8 0 0 1-2.8 2.8H9.6L4 21.2v-3.6A2.8 2.8 0 0 1 3.2 15V7.8A2.8 2.8 0 0 1 6 5h11.2A2.8 2.8 0 0 1 20 7.8Z"/>',
  files: '<path d="M3.4 7.6A2.1 2.1 0 0 1 5.5 5.5h3.2a2 2 0 0 1 1.5.7l1.1 1.3h7.2a2.1 2.1 0 0 1 2.1 2.1v7.9a2.1 2.1 0 0 1-2.1 2.1H5.5a2.1 2.1 0 0 1-2.1-2.1Z"/>',
  folder: '<path d="M3.4 7.6A2.1 2.1 0 0 1 5.5 5.5h3.2a2 2 0 0 1 1.5.7l1.1 1.3h7.2a2.1 2.1 0 0 1 2.1 2.1v7.9a2.1 2.1 0 0 1-2.1 2.1H5.5a2.1 2.1 0 0 1-2.1-2.1Z"/>',
  settings: '<path d="M4 8.2h8.6M17.4 8.2H20M4 15.8h3.2M12 15.8h8"/><circle cx="15" cy="8.2" r="2.3"/><circle cx="9.6" cy="15.8" r="2.3"/>',
  plus: '<path d="M12 5.5v13M5.5 12h13"/>',
  close: '<path d="m6.6 6.6 10.8 10.8M17.4 6.6 6.6 17.4"/>',
  arrow: '<path d="M4.8 12h13.4M12.6 6.2 18.4 12l-5.8 5.8"/>',
  clip: '<path d="M18.6 11.4 11 19a4.5 4.5 0 0 1-6.4-6.4l7.6-7.6a3 3 0 0 1 4.3 4.3l-7.6 7.6a1.5 1.5 0 0 1-2.1-2.1l6.9-6.9"/>',
  shield: '<path d="m12 3.6 7.4 2.7v5.2c0 4.5-7.4 8.4-7.4 8.4s-7.4-3.9-7.4-8.4V6.3Z"/><path d="m9.2 11.9 2.1 2.1 3.9-4.3"/>',
  upload: '<path d="M12 16.4V4.6M7.6 9 12 4.6 16.4 9M4.6 19.4h14.8"/>',
  download: '<path d="M12 4.6v11.8M7.6 12 12 16.4 16.4 12M4.6 19.4h14.8"/>',
  check: '<path d="m5.4 12.4 4.4 4.4L18.6 7.4"/>',
  info: '<circle cx="12" cy="12" r="8.6"/><path d="M12 11.2v5.4M12 8.1v.1"/>',
  alert: '<path d="M12 4.6 20.8 19.6H3.2Z"/><path d="M12 10.2v4.2M12 16.7v.1"/>',
  copy: '<rect x="8.4" y="8.4" width="10.2" height="10.2" rx="2"/><path d="M15.6 8.4V6.2a1.8 1.8 0 0 0-1.8-1.8H6.2a1.8 1.8 0 0 0-1.8 1.8v7.6a1.8 1.8 0 0 0 1.8 1.8h2.2"/>',
  key: '<circle cx="8.2" cy="12" r="3.6"/><path d="M11.8 12H20M17.2 12v3.2M14.4 12v2.2"/>',
  play: '<path d="M8 5.4v13.2L19 12Z"/>',
  pause: '<path d="M8.6 5.4v13.2M15.4 5.4v13.2"/>',
  palette: '<path d="M12 3.6a8.4 8.4 0 1 0 0 16.8c1.2 0 1.9-.8 1.9-1.8 0-1.1-.8-1.7-.8-2.7 0-.9.7-1.5 1.7-1.5h1.6a4.3 4.3 0 0 0 4.3-4.3c0-3.6-3.9-6.5-8.7-6.5Z"/><circle cx="8.4" cy="10.2" r="1.1"/><circle cx="12" cy="8" r="1.1"/><circle cx="15.6" cy="10" r="1.1"/>',
  trash: '<path d="M5.4 7.6h13.2M10 7.6V5.8h4v1.8M7.2 7.6l1 11h7.6l1-11"/>',
  file: '<path d="M13.4 4.4H7.2a1.8 1.8 0 0 0-1.8 1.8v11.6a1.8 1.8 0 0 0 1.8 1.8h9.6a1.8 1.8 0 0 0 1.8-1.8V9.4Z"/><path d="M13.4 4.4v5h5"/>',
};

const icon = (name: string) =>
  `<svg class="i" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${paths[name] ?? paths.files}</svg>`;

const esc = (text: string) =>
  text.replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]!));

const $ = <T extends Element = HTMLElement>(selector: string) => document.querySelector<T>(selector)!;
const $$ = <T extends Element = HTMLElement>(selector: string) => Array.from(document.querySelectorAll<T>(selector));

const bytes = (n: number) => {
  const units = ['B', 'KB', 'MB', 'GB', 'TB'];
  let i = 0;
  while (n >= 1024 && i < 4) {
    n /= 1024;
    i++;
  }
  return `${n.toFixed(i && n < 10 ? 1 : 0)} ${units[i]}`;
};

const clock = (timestamp: number) =>
  new Date(timestamp).toLocaleTimeString('zh-CN', { hour: '2-digit', minute: '2-digit' });

const brandMark = `<svg class="brand-mark" viewBox="0 0 28 28" fill="none" aria-hidden="true" focusable="false">
  <rect x="2.4" y="6.6" width="8.6" height="14.8" rx="3.2" fill="currentColor"/>
  <rect x="17" y="6.6" width="8.6" height="14.8" rx="3.2" fill="currentColor" fill-opacity=".32"/>
  <path d="M11 14h6" stroke="currentColor" stroke-width="1.7" stroke-linecap="round"/>
</svg>`;

/* connection motif: two devices and the link between them, drawn per state */
const motif = (mode: 'disconnected' | 'connecting' | 'connected') =>
  `<svg class="motif" data-state="${mode}" viewBox="0 0 208 88" fill="none" aria-hidden="true" focusable="false">
  <rect class="m-device" x="12" y="16" width="56" height="56" rx="12"/>
  <rect class="m-bar" x="25" y="31" width="30" height="5" rx="2.5"/>
  <rect class="m-bar" x="25" y="42" width="19" height="5" rx="2.5"/>
  <rect class="m-bar" x="25" y="53" width="24" height="5" rx="2.5"/>
  <rect class="m-device" x="140" y="16" width="56" height="56" rx="12"/>
  <rect class="m-bar" x="153" y="31" width="30" height="5" rx="2.5"/>
  <rect class="m-bar" x="153" y="42" width="19" height="5" rx="2.5"/>
  <rect class="m-bar" x="153" y="53" width="24" height="5" rx="2.5"/>
  <path class="m-link" d="M68 44h72" stroke-linecap="round"/>
  <circle class="m-node" cx="86" cy="44" r="4.4"/>
  <circle class="m-node" cx="122" cy="44" r="4.4"/>
  <circle class="m-node-core" cx="86" cy="44" r="1.6"/>
  <circle class="m-node-core" cx="122" cy="44" r="1.6"/>
</svg>`;

/* ---------- state ---------- */

let state: Snapshot = {
  profile: { displayName: '我的电脑', downloadDir: '', autoAcceptFiles: false },
  connection: 'disconnected',
  peerName: null,
  messages: [],
  transfers: [],
  engineAvailable: false,
  preview: !api.isDesktop(),
  paused: false,
  reconnect: null,
  server: null,
  room: null,
  route: 'unknown',
  mailbox: { state: 'notConfigured', message: null, pending: 0 },
};

let currentPage: 'chat' | 'files' = 'chat';
let sending = false;
let preparing = false;
let pairBusy = false;
let ready = false;
const pending: AppEvent[] = [];
let toastTimer: ReturnType<typeof setTimeout> | undefined;

/* History paging. `roomEpoch` invalidates in-flight page loads when the room
 * changes, so a stale response can never pollute the new transcript. */
let viewRoomId: string | null = null;
let roomEpoch = 0;
let historyHasMore = false;
let historyLoading = false;
/* Last message id painted into the thread, so a receipt update is not mistaken
 * for an appended message. */
let renderedLastId: string | null = null;
let forceScroll = false;

const isHost = () => state.room?.role === 'host';
const retrySeconds = () => Math.max(1, Math.ceil((state.reconnect?.nextRetryMs ?? 0) / 1000));

/* ---------- shell ---------- */

$('#app').innerHTML = `
<div class="app">
  <aside class="sidebar">
    <button type="button" class="brand" data-page="chat" aria-label="同频，回到会话">
      ${brandMark}
      <span class="brand-text"><span class="brand-name">同频</span><span class="brand-sub">TwinLink</span></span>
    </button>
    <nav class="nav" aria-label="主导航">
      <button type="button" class="nav-item active" data-page="chat" aria-current="page" aria-label="会话" title="会话">${icon('chat')}<span>会话</span></button>
      <button type="button" class="nav-item" data-page="files" aria-label="文件传输" title="文件传输">${icon('files')}<span>文件传输</span><b class="nav-badge" aria-hidden="true" hidden>0</b></button>
    </nav>
    <div class="sidebar-grow"></div>
    <section class="peer" data-state="disconnected" aria-label="连接状态">
      <p class="peer-label">对方</p>
      <div class="peer-name"><i class="peer-dot"></i><span class="peer-title">未连接</span></div>
      <p class="peer-desc">配对后，消息会加密暂存在离线信箱，对方上线后收取。</p>
      <div class="mailbox" data-state="notConfigured" aria-live="polite">
        <i class="mailbox-dot" aria-hidden="true"></i>
        <span class="mailbox-text">离线信箱未配置</span>
      </div>
      <button type="button" class="link-btn room-open peer-room" hidden>房间详情</button>
    </section>
    <div class="side-foot">
      <button type="button" class="nav-item appearance-open" aria-label="外观" title="外观与主题">${icon('palette')}<span>外观</span></button>
      <button type="button" class="nav-item settings-open" aria-label="设置" title="设置">${icon('settings')}<span>设置</span></button>
      <div class="device">
        <span class="avatar" aria-hidden="true">我</span>
        <span class="device-text"><span class="device-name">我的电脑</span><span class="device-sub">本机设备</span></span>
      </div>
    </div>
  </aside>
  <main class="workspace">
    <header class="topbar">
      <div class="topbar-title">
        <h1 id="page-title">会话</h1>
        <p class="topbar-sub" id="page-sub">配对后开始对话</p>
      </div>
      <div class="topbar-actions">
        <span class="pill mailbox-pill" data-state="notConfigured" title="离线信箱状态"><i class="pill-dot"></i><span class="mailbox-pill-text">信箱未配置</span></span>
        <span class="pill peer-pill" data-state="disconnected" title="对方在线状态"><i class="pill-dot"></i><span class="pill-text">未连接</span></span>
        <button type="button" class="btn quiet room-open" hidden>${icon('key')}房间</button>
        <button type="button" class="btn primary conn-action">${icon('plus')}开始配对</button>
      </div>
    </header>
    <div class="preview-strip" hidden>${icon('info')}<strong>界面预览</strong><span>当前在浏览器中查看。创建房间、配对、聊天与文件传输需在桌面应用中进行。</span></div>
    <div class="reconnect-strip" hidden role="status" aria-live="polite">
      ${icon('info')}
      <span class="reconnect-text"></span>
      <button type="button" class="btn quiet small resume-action" hidden>${icon('play')}立即重连</button>
    </div>
    <section class="page page-chat" aria-label="会话">
      <div class="thread" role="log" aria-label="聊天记录" aria-live="polite" aria-relevant="additions text">
        <div class="thread-inner">
          <button type="button" class="history-more" hidden>加载更早的消息</button>
          <div class="empty-slot"></div>
          <ol class="messages" hidden></ol>
        </div>
      </div>
      <div class="composer-zone"><div class="composer-zone-inner">
        <form class="composer" id="composer">
          <label class="sr-only" for="message-input">消息内容</label>
          <textarea id="message-input" rows="2" maxlength="16000" placeholder="连接后可以发送消息"></textarea>
          <div class="composer-bar">
            <button type="button" class="icon-btn attach-button" aria-label="选择文件发送" title="选择文件发送" disabled>${icon('clip')}</button>
            <span class="composer-hint">Enter 发送，Shift + Enter 换行</span>
            <button type="submit" class="btn primary send-button" disabled>发送${icon('arrow')}</button>
          </div>
        </form>
        <p class="composer-note">
          <span>${icon('shield')}直连在线时先询问接收，文件通过 P2P 传输；离线时加密暂存</span>
          <button type="button" class="link-btn show-files">查看文件传输</button>
        </p>
      </div></div>
    </section>
    <section class="page page-files" hidden aria-label="文件传输">
      <div class="files-inner">
        <div class="files-head">
          <div>
            <h2>文件传输</h2>
            <p>直连在线时，接受后通过 P2P 传输，无应用文件大小限制；无法直连时加密暂存服务器，保留 7 天、单文件最多 1 GiB。同名自动改名。</p>
          </div>
          <button type="button" class="btn primary choose-file" disabled>${icon('plus')}发送文件</button>
        </div>
        <div class="transfers" aria-live="polite"></div>
        <div class="files-empty">
          <div class="files-empty-inner">
            <div class="files-empty-icon" aria-hidden="true">${icon('folder')}</div>
            <h3 class="files-empty-title">还没有传输文件</h3>
            <p class="files-empty-copy">选择或拖入文件发送。直连在线时先询问接收，离线时加密暂存服务器。</p>
          </div>
        </div>
        <div class="files-foot">
          ${icon('folder')}<span class="download-path">桌面应用中的下载目录</span>
          <button type="button" class="link-btn settings-open">更改</button>
        </div>
      </div>
    </section>
  </main>
</div>

<dialog class="dialog" id="dlg-pair" aria-labelledby="pair-title">
  <form class="dialog-form" id="pair-form" novalidate>
    <div class="dialog-head">
      <div>
        <h2 id="pair-title">配对两台电脑</h2>
        <p>创建房间后把配对码发给对方，或粘贴对方给你的配对码加入。配对码就是房间钥匙。</p>
      </div>
      <button type="button" class="icon-btn dialog-close" aria-label="关闭">${icon('close')}</button>
    </div>
    <div class="pair-choose">
      <fieldset class="role-pick">
        <legend class="sr-only">配对方式</legend>
        <label>
          <input type="radio" name="mode" value="create" checked />
          <span class="role-tile"><strong>创建房间</strong><small>生成配对码，等对方加入</small></span>
        </label>
        <label>
          <input type="radio" name="mode" value="join" />
          <span class="role-tile"><strong>输入配对码加入</strong><small>连接已在等待的伙伴</small></span>
        </label>
      </fieldset>
      <div class="pair-create">
        <p class="pair-copy">创建后，本机会等待对方加入。把生成的配对码通过可信渠道发给对方即可。</p>
        <p class="note pair-server-hint" hidden>${icon('alert')}<span>还没有导入服务器设置码。请先在设置中导入管理员提供的设置码。</span></p>
        <button type="button" class="btn primary create-room">${icon('plus')}创建房间</button>
      </div>
      <div class="pair-join" hidden>
        <label class="field">
          <span class="field-label">配对码</span>
          <textarea class="input code-input" name="invite" rows="3" placeholder="TL1.…" autocomplete="off" spellcheck="false"></textarea>
          <span class="field-hint">配对码包含房间密钥，等同于进入这个房间的钥匙。请勿发到公开群组或截图分享。</span>
        </label>
      </div>
    </div>
    <div class="pair-key-panel" hidden>
      <h3>房间已创建</h3>
      <p>把下面的配对码发给对方。对方在“输入配对码加入”中粘贴后，就会进入同一个房间。</p>
      <label class="field">
        <span class="field-label">配对码</span>
        <textarea class="input code-input pairing-code" readonly rows="4" aria-label="房间配对码"></textarea>
      </label>
      <div class="copy-row">
        <button type="button" class="btn primary copy-key">${icon('copy')}复制配对码</button>
        <button type="button" class="btn quiet done-pair">完成</button>
      </div>
      <p class="field-hint">配对码不会被写入聊天记录。任何拿到它的人都能加入此房间，请只发给你信任的人。</p>
    </div>
    <p class="form-error" role="alert"></p>
    <div class="dialog-foot pair-foot">
      <button type="button" class="btn quiet dialog-close">取消</button>
      <button type="submit" class="btn primary pair-submit" hidden>加入房间</button>
    </div>
  </form>
</dialog>

<dialog class="dialog dialog-wide" id="dlg-room" aria-labelledby="room-title">
  <div class="dialog-form">
    <div class="dialog-head">
      <div>
        <h2 id="room-title">房间详情</h2>
        <p>这些信息只在本机生成和保存，配对码中的房间密钥不会显示在这里。</p>
      </div>
      <button type="button" class="icon-btn dialog-close" aria-label="关闭">${icon('close')}</button>
    </div>
    <dl class="room-grid">
      <dt>角色</dt><dd id="room-role">—</dd>
      <dt>房间标识</dt><dd id="room-id" class="mono"></dd>
      <dt>网络名称</dt><dd id="room-network" class="mono"></dd>
      <dt>本机虚拟地址</dt><dd id="room-ip" class="mono"></dd>
      <dt>对方虚拟地址</dt><dd id="room-peer-ip" class="mono"></dd>
      <dt>连接方式</dt><dd id="room-route">未知</dd>
      <dt>离线信箱</dt><dd id="room-mailbox">—</dd>
      <dt>服务器</dt><dd id="room-server" class="mono"></dd>
    </dl>
    <p class="field-hint">本机与对方的虚拟地址根据房间密钥自动分配，无需手动填写。实时通道仅允许 P2P 直连；无法直连时使用离线信箱，不中继文件字节。</p>
    <div class="room-key-block" hidden>
      <button type="button" class="btn quiet copy-key">${icon('copy')}复制配对码</button>
      <span class="field-hint">重新复制本房间的配对码，分享给要加入的伙伴。</span>
      <textarea class="input code-input room-key" readonly rows="3" hidden aria-label="房间配对码"></textarea>
    </div>
    <p class="form-error" role="alert"></p>
    <div class="dialog-foot room-foot">
      <button type="button" class="btn quiet forget-room">忘记这个房间</button>
      <span class="foot-grow"></span>
      <button type="button" class="btn quiet dialog-close">关闭</button>
    </div>
    <p class="field-hint forget-hint">忘记房间只会清除保存的配对信息，聊天记录仍会保留在本机。</p>
  </div>
</dialog>

<dialog class="dialog" id="dlg-settings" aria-labelledby="settings-title">
  <form class="dialog-form" id="settings-form" novalidate>
    <div class="dialog-head">
      <div>
        <h2 id="settings-title">设置</h2>
        <p>设备名称会显示给对方，下载目录决定收到的文件保存位置。</p>
      </div>
      <button type="button" class="icon-btn dialog-close" aria-label="关闭">${icon('close')}</button>
    </div>
    <label class="field">
      <span class="field-label">设备名称</span>
      <input class="input" name="displayName" required maxlength="64" autocomplete="off" />
    </label>
    <label class="field">
      <span class="field-label">文件保存位置</span>
      <span class="directory-row">
        <input class="input" name="downloadDir" readonly placeholder="尚未选择目录" />
        <button type="button" class="btn quiet choose-directory">${icon('folder')}选择</button>
      </span>
      <span class="field-hint">同名文件自动改名，不会覆盖已有文件。</span>
    </label>

    <label class="field auto-receive-field">
      <span><input type="checkbox" name="autoAcceptFiles" /> 自动接收当前房间的文件</span>
      <span class="field-hint">默认关闭，更换房间后关闭。开启后，新文件请求会直接接收并保存到上方目录；不会自动打开或执行文件。</span>
    </label>

    <section class="dialog-section" aria-labelledby="appearance-heading">
      <h3 id="appearance-heading">外观与主题</h3>
      <p class="field-hint">明暗模式、强调色、正文字号与可安装的 JSON 主题包都在独立的外观面板中设置，不影响连接与记录。</p>
      <div class="copy-row">
        <button type="button" class="btn quiet open-appearance">打开外观与主题插件</button>
      </div>
    </section>

    <section class="dialog-section" aria-labelledby="backup-heading">
      <h3 id="backup-heading">数据导入与导出</h3>
      <p class="field-hint">备份包含全部聊天记录、传输记录和房间／服务器凭据，未加密，请妥善保存。附件本体及未完成的发送队列不包含在内。</p>
      <p class="field-hint">导入会合并记录、保留本机已有房间和保存目录，暂停连接并关闭自动接收；空白设备可恢复备份中的房间。</p>
      <div class="copy-row">
        <button type="button" class="btn quiet" id="export-data">导出备份</button>
        <button type="button" class="btn quiet" id="import-data">选择备份导入</button>
        <button type="button" class="btn quiet" id="cancel-import" hidden>取消导入</button>
      </div>
      <p class="field-hint" id="backup-status" role="status" aria-live="polite"></p>
    </section>

    <section class="dialog-section" aria-labelledby="server-heading">
      <h3 id="server-heading">联机服务器</h3>
      <div id="server-config" hidden>
        <div class="server-card">
          <p><span class="server-key">中继</span><span id="server-relay" class="mono"></span></p>
          <p><span class="server-key">信箱</span><span id="server-mailbox" class="mono"></span></p>
        </div>
      </div>
      <p class="field-hint" id="server-none">尚未导入服务器设置码。</p>
      <label class="field">
        <span class="field-label">服务器设置码</span>
        <textarea class="input code-input" name="serverCode" id="server-code" rows="3" placeholder="TL-SRV1.…" autocomplete="off" spellcheck="false"></textarea>
        <span class="field-hint">设置码由管理员提供，包含服务器访问令牌，属于凭据。导入后只保存在本机，不会显示在快照或日志中。</span>
      </label>
      <div class="copy-row">
        <button type="button" class="btn quiet import-server">导入服务器设置码</button>
      </div>
      <p class="form-error" id="server-error" role="alert"></p>
    </section>

    <section class="dialog-section" aria-labelledby="update-heading">
      <h3 id="update-heading">应用更新 <span class="update-version"></span></h3>
      <p class="field-hint" id="update-status" role="status" aria-live="polite">更新源固定为 GitHub。检查更新后，由你决定是否安装。</p>
      <p class="field-hint update-notes" id="update-notes" hidden></p>
      <progress id="update-progress" max="1" hidden aria-label="应用更新下载进度"></progress>
      <div class="copy-row">
        <button type="button" class="btn quiet" id="check-update">检查更新</button>
        <button type="button" class="btn primary" id="install-update" hidden>更新并重启</button>
      </div>
    </section>

    <section class="dialog-section" aria-labelledby="lan-heading">
      <h3 id="lan-heading">局域网游戏模式</h3>
      <div class="feature-off">
        <p><strong>未完成</strong></p>
        <p>该模式需要真实的 TUN 虚拟网卡与管理员授权，当前版本不提供。配对连接不会把本机变成局域网，也不会修改系统防火墙或向对方暴露磁盘和子网。</p>
      </div>
    </section>

    <p class="form-error" role="alert"></p>
    <div class="dialog-foot">
      <button type="button" class="btn quiet dialog-close">取消</button>
      <button type="submit" class="btn primary">保存设置</button>
    </div>
  </form>
</dialog>

<div id="file-drop-hint" class="file-drop-hint" hidden>松开发送所选文件</div>
<div class="toast" role="status" aria-live="polite" hidden></div>

<dialog class="dialog" id="dlg-appearance" aria-labelledby="appearance-title">
  <div class="dialog-form">
    <div class="dialog-head">
      <div>
        <h2 id="appearance-title">外观</h2>
        <p>主题插件是声明式 JSON 配色包，只包含颜色、圆角和字号，不执行任何代码。</p>
      </div>
      <button type="button" class="icon-btn dialog-close" aria-label="关闭">${icon('close')}</button>
    </div>
    <section class="dialog-section" aria-labelledby="appearance-mode-heading">
      <h3 id="appearance-mode-heading">明暗模式</h3>
      <div class="segmented" role="group" aria-label="明暗模式">
        <button type="button" class="segmented-item" data-mode="light">浅色</button>
        <button type="button" class="segmented-item" data-mode="dark">深色</button>
        <button type="button" class="segmented-item" data-mode="system">跟随系统</button>
      </div>
      <p class="field-hint" id="appearance-mode-hint" role="status" aria-live="polite"></p>
    </section>
    <section class="dialog-section" aria-labelledby="appearance-theme-heading">
      <h3 id="appearance-theme-heading">主题</h3>
      <div class="theme-grid" id="theme-grid" role="radiogroup" aria-label="主题"></div>
      <p class="field-hint" id="theme-status" role="status" aria-live="polite"></p>
    </section>
    <section class="dialog-section" aria-labelledby="appearance-tune-heading">
      <h3 id="appearance-tune-heading">强调色与字号</h3>
      <div class="appearance-row">
        <label class="appearance-accent">
          <span class="field-label">强调色</span>
          <span class="accent-row">
            <input type="color" id="accent-input" aria-label="自定义强调色" />
            <button type="button" class="btn quiet small" id="accent-clear">恢复主题强调色</button>
          </span>
        </label>
        <label class="appearance-size">
          <span class="field-label">界面字号 <b id="font-size-value" class="mono"></b></span>
          <input type="range" id="font-size-input" min="12" max="18" step="1" aria-label="界面字号" />
        </label>
      </div>
    </section>
    <section class="dialog-section" aria-labelledby="appearance-plugin-heading">
      <h3 id="appearance-plugin-heading">主题插件</h3>
      <p class="field-hint">导出会生成一个小型 JSON 文件，可以修改颜色后重新导入。也可以只复制文本自己保存。</p>
      <div class="copy-row">
        <button type="button" class="btn quiet" id="theme-import">导入主题包文件</button>
        <button type="button" class="btn quiet" id="theme-export">导出当前主题</button>
        <button type="button" class="btn quiet" id="theme-copy">复制主题 JSON</button>
        <button type="button" class="btn quiet" id="theme-reset">恢复默认外观</button>
      </div>
      <input type="file" id="theme-file" accept="application/json,.json" hidden />
      <p class="field-hint" id="theme-plugin-status" role="status" aria-live="polite"></p>
      <div class="theme-confirm" id="theme-confirm" role="alertdialog" aria-labelledby="theme-confirm-message" hidden>
        <p class="theme-confirm-text" id="theme-confirm-message"></p>
        <div class="theme-confirm-actions">
          <button type="button" class="btn quiet" id="theme-confirm-cancel">取消</button>
          <button type="button" class="btn danger" id="theme-confirm-accept">删除主题</button>
        </div>
      </div>
      <label class="field">
        <span class="field-label">主题 JSON 文本</span>
        <textarea class="input code-input" id="theme-json" rows="6" spellcheck="false" autocomplete="off" placeholder="导出的主题包会显示在这里，也可以粘贴主题包 JSON 后点“应用这份 JSON”。"></textarea>
        <span class="field-hint">粘贴的主题包会先校验格式、颜色和大小（上限 64 KiB），通过后才会安装。</span>
      </label>
      <div class="copy-row">
        <button type="button" class="btn quiet" id="theme-apply">应用这份 JSON</button>
      </div>
      <p class="form-error" id="theme-error" role="alert"></p>
    </section>
    <div class="dialog-foot">
      <button type="button" class="btn quiet dialog-close">关闭</button>
    </div>
  </div>
</dialog>`;

/* ---------- rendering ---------- */

function toast(message: string) {
  const el = $('.toast');
  el.textContent = message;
  el.hidden = false;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => (el.hidden = true), 6500);
}

function showPage(page: 'chat' | 'files') {
  currentPage = page;
  $('.page-chat').hidden = page !== 'chat';
  $('.page-files').hidden = page !== 'files';
  $$('[data-page]').forEach(button => {
    const active = (button as HTMLElement).dataset.page === page;
    button.classList.toggle('active', active);
    if (active) button.setAttribute('aria-current', 'page');
    else button.removeAttribute('aria-current');
  });
  renderChrome();
  renderTransfers();
  if (page === 'chat') $<HTMLTextAreaElement>('#message-input').focus({ preventScroll: true });
}

function emptyMarkup(): string {
  const engineReady = state.engineAvailable || state.preview;
  const role = state.room?.role;
  const paused = state.paused;
  const mailboxOnline = state.mailbox.state === 'online';
  /* Say what actually happens: a durable encrypted mailbox copy, not a promise
   * that the other person is present. */
  const channel = mailboxOnline
    ? '对方不在线时，消息会先加密暂存在离线信箱，等对方上线后收取。'
    : '离线信箱当前不可用，消息会先保存在本机，恢复后自动发送。';
  const hostCopy =
    role === 'host'
      ? '正在准备联机组件，等待对方粘贴你的配对码加入。'
      : '正在准备联机组件并加入同一个房间。';

  if (state.connection === 'connected') {
    return `<div class="empty" data-state="connected">
      ${motif('connected')}
      <h2>你们已经连接</h2>
      <p>消息会直接送达对方，同时加密暂存到离线信箱；对方离线时也能收到。发一句话，或者选择一个文件发送。</p>
    </div>`;
  }

  if (state.connection === 'connecting') {
    return `<div class="empty" data-state="connecting">${motif('connecting')}<h2>${
      paused ? '连接已暂停' : role === 'host' ? '正在等待对方加入' : role === 'join' ? '正在连接对方' : '正在连接'
    }</h2><p>${paused ? '你手动断开了连接。房间和聊天记录都还在，随时可以重新连接。' : `${hostCopy}${channel}`}</p>${
      paused ? '<button type="button" class="btn primary resume-action">重新连接</button>' : ''
    }</div>`;
  }

  if (state.room) {
    return `<div class="empty" data-state="disconnected">
      ${motif('disconnected')}
      <h2>${paused ? '已手动断开' : '房间已保存'}</h2>
      <p>${
        paused
          ? '自动重连已暂停。房间、虚拟地址和聊天记录都还保留着；你仍可以输入消息，它们会先留在本机，重新连接后发出。'
          : `本机保存了这个房间，会自动尝试重新连接。${channel}`
      }</p>
      ${engineReady ? '' : '<p class="empty-aside">本机没有找到联机组件 EasyTier，暂时无法直连；离线信箱仍可收发消息。</p>'}
      <button type="button" class="btn primary resume-action">${icon('play')}重新连接</button>
      <button type="button" class="btn quiet open-room">查看房间详情</button>
    </div>`;
  }

  return `<div class="empty" data-state="disconnected">
    ${motif('disconnected')}
    <h2>还没有配对</h2>
    <p>同频只在一对电脑之间使用。创建房间并把配对码发给对方，或者粘贴对方给你的配对码。</p>
    ${engineReady ? '' : '<p class="empty-aside">本机没有找到联机组件 EasyTier，暂时无法直连。</p>'}
    <button type="button" class="btn primary open-pair">${icon('key')}开始配对</button>
    <button type="button" class="btn quiet settings-trigger">导入服务器设置码</button>
  </div>`;
}

/* How far an outgoing message has actually got. `stored` is never "delivered":
 * a server-side encrypted copy is not peer acknowledgement. Legacy rows with no
 * receipt metadata stay `unknown` and are never promoted. */
const deliveryLabels: Record<Message['delivery'], string> = {
  pending: '本机待发',
  stored: '服务器已存',
  delivered: '对方已收到',
  unknown: '未确认',
};

function messageMarkup(message: Message): string {
  const who = message.outgoing ? '我' : (state.peerName ?? '对方');
  const stamp = new Date(message.timestamp);
  const delivery = message.outgoing
    ? `<span class="msg-status" data-delivery="${esc(message.delivery)}" title="${
        message.via === 'direct' ? '通过联机直发' : message.via === 'mailbox' ? '通过离线信箱' : ''
      }">${deliveryLabels[message.delivery] ?? deliveryLabels.unknown}</span>`
    : '';
  return `<li class="msg ${message.outgoing ? 'out' : 'in'}" data-id="${esc(message.id)}">
    <div class="msg-meta"><span class="msg-who">${esc(who)}</span><time datetime="${stamp.toISOString()}">${clock(message.timestamp)}</time>${delivery}</div>
    <div class="msg-bubble">${esc(message.text)}</div>
  </li>`;
}

/** Sync the paging window to the room currently shown. Called from renderAll,
 * so a room swap through any path (state event, resync, refresh) resets it. */
function syncRoomWindow() {
  const id = state.room?.roomId ?? null;
  if (id === viewRoomId) return;
  viewRoomId = id;
  roomEpoch += 1;
  historyLoading = false;
  historyHasMore = state.messages.length >= 200;
  /* A different room starts a different transcript: land on the newest page. */
  renderedLastId = null;
  forceScroll = true;
}

function renderHistoryButton() {
  const button = $<HTMLButtonElement>('.history-more');
  /* Only offer paging when the transcript is actually windowed. */
  const visible = !!state.messages.length && historyHasMore;
  button.hidden = !visible;
  button.disabled = historyLoading;
  button.textContent = historyLoading ? '正在加载…' : '加载更早的消息';
}

function renderThread(options: { keepAnchor?: { height: number; top: number } } = {}) {
  const thread = $('.thread');
  const nearBottom = thread.scrollHeight - thread.scrollTop - thread.clientHeight < 96;
  const list = $<HTMLOListElement>('.messages');
  const empty = $('.empty-slot');

  if (state.messages.length) {
    empty.hidden = true;
    empty.innerHTML = '';
    list.hidden = false;
    list.innerHTML = state.messages.map(messageMarkup).join('');
  } else {
    empty.hidden = false;
    empty.innerHTML = emptyMarkup();
    list.hidden = true;
    list.innerHTML = '';
  }
  renderHistoryButton();

  /* Follow only a genuinely appended message, never a delivery-state update:
   * otherwise a receipt landing would yank a reader out of older history. */
  const lastId = state.messages.at(-1)?.id ?? null;
  const appended = lastId !== null && lastId !== renderedLastId;
  renderedLastId = lastId;

  if (options.keepAnchor) {
    /* Preserve the reader's place after prepending older messages. */
    thread.scrollTop = options.keepAnchor.top + (thread.scrollHeight - options.keepAnchor.height);
    forceScroll = false;
  } else if (forceScroll || nearBottom || (appended && state.messages.at(-1)?.outgoing)) {
    thread.scrollTop = thread.scrollHeight;
    forceScroll = false;
  }
}

async function loadEarlier() {
  if (historyLoading || !historyHasMore) return;
  const before = state.messages[0]?.id;
  if (!before || !viewRoomId) return;
  const epoch = roomEpoch;
  const room = viewRoomId;
  const thread = $('.thread');
  const anchor = { height: thread.scrollHeight, top: thread.scrollTop };
  historyLoading = true;
  renderHistoryButton();
  try {
    const page = await api.history(before);
    /* A room switch while this request was in flight invalidates the result. */
    if (epoch !== roomEpoch || page.roomId !== room) return;
    /* A paged result is a snapshot from when the server read it: its newest row
     * may overlap the loaded window with an older delivery state. Keep the local
     * copy for known ids (a receipt may have landed meanwhile) and prepend only
     * messages that are genuinely new. */
    const known = new Set(state.messages.map(message => message.id));
    const older = page.messages.filter(message => !known.has(message.id));
    state.messages = older.concat(state.messages);
    historyHasMore = page.hasMore;
    renderThread({ keepAnchor: anchor });
  } catch (error) {
    toast(`无法加载更早的消息：${String(error)}`);
  } finally {
    if (epoch === roomEpoch) {
      historyLoading = false;
      renderHistoryButton();
    }
  }
}

const statusLabels: Record<Transfer['status'], string> = {
  preparing: '准备中',
  uploading: '离线暂存上传中',
  sending: 'P2P 发送中',
  waiting: '等待对方接收',
  offered: '等待你接收',
  accepted: '已接受',
  downloading: '接收中',
  publishing: '正在保存',
  completed: '已完成',
  rejected: '已拒收',
  cancelled: '已取消',
  failed: '传输失败',
};

const ACTIVE_STATUSES: Transfer['status'][] = [
  'preparing',
  'uploading',
  'sending',
  'waiting',
  'offered',
  'accepted',
  'downloading',
  'publishing',
];
const SHOW_PROGRESS: Transfer['status'][] = [
  'preparing',
  'uploading',
  'sending',
  'accepted',
  'downloading',
  'publishing',
];

function transferMarkup(transfer: Transfer): string {
  const percent = transfer.size
    ? Math.min(100, Math.round((transfer.transferred / transfer.size) * 100))
    : transfer.status === 'completed'
      ? 100
      : 0;
  const tone =
    transfer.status === 'failed'
      ? 'danger'
      : ACTIVE_STATUSES.includes(transfer.status)
        ? 'active'
        : '';
  /* Accepting/rejecting a staged file needs only the mailbox server, not a live
   * peer. Uploads can be cancelled while fully staged (`waiting`). */
  const incoming = !transfer.outgoing;
  const canCancel = transfer.outgoing
    ? ['preparing', 'uploading', 'sending', 'waiting'].includes(transfer.status)
    : ['accepted', 'downloading', 'waiting'].includes(transfer.status);
  const canReject = incoming && transfer.status === 'offered';
  const actions = [
    canReject
      ? `<button type="button" class="btn primary small" data-accept="${esc(transfer.id)}">接收</button>
         <button type="button" class="btn quiet small" data-reject="${esc(transfer.id)}">拒收</button>`
      : '',
    canCancel
      ? `<button type="button" class="btn quiet small" data-cancel="${esc(transfer.id)}">${
          transfer.outgoing ? '取消发送' : '取消接收'
        }</button>`
      : '',
    transfer.status === 'completed'
      ? `<span class="transfer-done" role="img" aria-label="传输完成">${icon('check')}</span>`
      : '',
  ]
    .filter(Boolean)
    .join('');

  /* Consent is the point of an incoming offer: nothing is written to disk until
   * the recipient accepts, so say that on the card itself. */
  const hint =
    transfer.status === 'offered' && incoming
      ? '<p class="transfer-hint">只有你点“接收”后才会保存到本机；拒收不会下载这个文件。</p>'
      : transfer.status === 'failed' && transfer.outgoing
      ? '<p class="transfer-hint">文件未能完成发送。请重新选择这个文件发送。</p>'
      : transfer.status === 'failed' && incoming
        ? '<p class="transfer-hint">文件未能接收完成。请让对方重新发送，或再次接收。</p>'
        : '';

  return `<article class="transfer" data-status="${transfer.status}">
    <div class="transfer-glyph" aria-hidden="true">${icon(transfer.outgoing ? 'upload' : 'download')}</div>
    <div class="transfer-body">
      <p class="transfer-name" title="${esc(transfer.name)}">${esc(transfer.name)}</p>
      <p class="transfer-meta">
        <span>${transfer.outgoing ? '你发送' : '对方发送'}</span>
        <span>${bytes(transfer.size)}</span>
        <span>${transfer.via === 'direct' ? 'P2P 直连' : '离线暂存'}</span>
        <span class="chip" data-tone="${tone}">${statusLabels[transfer.status] ?? transfer.status}${
          SHOW_PROGRESS.includes(transfer.status) ? ` ${percent}%` : ''
        }</span>
      </p>
      ${
        SHOW_PROGRESS.includes(transfer.status)
          ? `<progress value="${percent}" max="100" aria-label="${esc(transfer.name)} 的传输进度"></progress>`
          : ''
      }
      ${transfer.error ? `<p class="transfer-error">${esc(transfer.error)}</p>` : ''}
      ${hint}
    </div>
    <div class="transfer-actions">${actions}</div>
  </article>`;
}

function renderTransfers() {
  const list = $('.transfers');
  const active = state.transfers.filter(transfer => ACTIVE_STATUSES.includes(transfer.status)).length;
  const badge = $('.nav-badge');
  badge.textContent = String(active);
  badge.hidden = active === 0;

  $('.files-empty').hidden = state.transfers.length > 0;

  if (!state.transfers.length) {
    list.innerHTML = '';
    const readyToSend = !!state.room;
    $('.files-empty-title').textContent = readyToSend ? '还没有传输文件' : '配对后才能传输文件';
    $('.files-empty-copy').textContent = readyToSend
      ? '选择或拖入文件发送。直连在线时先询问接收，离线时加密暂存服务器。'
      : '先创建或加入一个房间，就可以把文件加密暂存到服务器，等对方收取。';
    return;
  }

  list.innerHTML = state.transfers
    .slice()
    .reverse()
    .map(transferMarkup)
    .join('');
}

function renderReconnect() {
  const strip = $('.reconnect-strip');
  const text = $('.reconnect-text');
  const resume = $<HTMLButtonElement>('.reconnect-strip .resume-action');
  const waiting = state.reconnect;
  const stopped = !state.paused && state.room !== null && state.connection === 'disconnected' && !waiting;

  if (state.paused && state.room) {
    strip.hidden = false;
    strip.dataset.tone = 'muted';
    text.textContent = '已手动断开。自动重连已暂停，房间和聊天记录仍保留在本机。';
    resume.hidden = false;
  } else if (waiting && state.connection !== 'connected') {
    strip.hidden = false;
    strip.dataset.tone = 'active';
    text.textContent = `连接中断，正在进行第 ${waiting.attempt} 次自动重连，约 ${retrySeconds()} 秒后重试。`;
    resume.hidden = false;
  } else if (stopped) {
    strip.hidden = false;
    strip.dataset.tone = 'muted';
    text.textContent = '自动重连已停止。你可以手动立即重试，或检查服务器与网络。';
    resume.hidden = false;
  } else {
    strip.hidden = true;
    resume.hidden = true;
  }
}

function renderChrome() {
  const connected = state.connection === 'connected';
  const connecting = state.connection === 'connecting';
  const peer = state.peerName ?? '对方';
  const role = state.room?.role;

  /* heading */
  $('#page-title').textContent = currentPage === 'chat' ? '会话' : '文件传输';
  $('#page-sub').textContent = connecting
    ? state.paused
      ? '连接已暂停'
      : role === 'host'
        ? '正在等待对方加入'
        : '正在准备连接'
    : connected
      ? currentPage === 'chat'
        ? `与 ${peer} 的会话`
        : `与 ${peer} 互传文件`
      : state.room && !state.paused
        ? '正在尝试重新连接'
        : currentPage === 'chat'
          ? '配对后开始对话'
          : '配对后可以传输文件';

  /* status pill */
  $('.peer-pill').setAttribute('data-state', state.paused ? 'disconnected' : state.connection);
  $('.pill-text').textContent = state.paused
    ? '已断开'
    : connected
      ? '已连接'
      : connecting
        ? state.reconnect
          ? `重连中 ${state.reconnect.attempt}`
          : '正在连接'
        : '未连接';

  /* room buttons */
  $$('.room-open').forEach(button => ((button as HTMLElement).hidden = !state.room));

  /* the single primary connection action */
  const action = $<HTMLButtonElement>('.conn-action');
  if (state.room && state.paused) {
    action.className = 'btn primary conn-action';
    action.innerHTML = `${icon('play')}重新连接`;
    action.setAttribute('aria-label', '重新连接已保存的房间');
  } else if (state.room && state.connection === 'disconnected') {
    action.className = 'btn primary conn-action';
    action.innerHTML = `${icon('play')}立即重连`;
    action.setAttribute('aria-label', '立即重新连接');
  } else if (connected || connecting) {
    action.className = 'btn quiet conn-action';
    action.innerHTML = `${icon('pause')}${connected ? '断开连接' : '取消连接'}`;
    action.setAttribute('aria-label', connected ? '断开连接并暂停自动重连' : '取消正在进行的连接');
  } else {
    action.className = 'btn primary conn-action';
    action.innerHTML = `${icon('key')}开始配对`;
    action.setAttribute('aria-label', '开始配对');
  }

  /* sidebar connection panel (informational only) */
  const panel = $('.peer');
  panel.setAttribute('data-state', state.paused ? 'disconnected' : state.connection);
  $('.peer-title').textContent = connected
    ? peer
    : state.paused
      ? '已断开'
      : state.reconnect
        ? `重连中 · ${state.reconnect.attempt}`
        : connecting
          ? '正在连接'
          : '未连接';
  $('.peer-desc').textContent = connected
    ? '已连接，可以聊天和传文件。'
    : state.paused
      ? '自动重连已暂停。消息会先保存在本机，恢复后发送。'
      : state.reconnect
        ? `连接中断，正在退避重试。消息会加密暂存，对方上线后收取。`
        : state.room
          ? '房间已保存。对方不在线时，消息和文件会加密暂存到服务器。'
          : '配对后，消息会加密暂存在离线信箱，对方上线后收取。';

  /* local device */
  $('.device-name').textContent = state.profile.displayName;
  $('.avatar').textContent = (state.profile.displayName.trim()[0] ?? '我').toUpperCase();
  const deviceSub = $('.device-sub');
  if (state.preview) {
    deviceSub.textContent = '浏览器预览';
    deviceSub.classList.remove('warn');
  } else if (!state.engineAvailable) {
    deviceSub.innerHTML = `${icon('alert')}联机组件缺失`;
    deviceSub.classList.add('warn');
  } else {
    deviceSub.textContent = '本机设备';
    deviceSub.classList.remove('warn');
  }

  /* misc */
  $('.preview-strip').hidden = !state.preview;
  $('.download-path').textContent = state.profile.downloadDir || '桌面应用中的下载目录';
  $('.pair-server-hint').hidden = state.server !== null;

  renderReconnect();
  renderMailbox();
  renderComposer();
}

/* Offline-mailbox reachability, shown separately from peer presence: the server
 * being reachable says nothing about whether the other person is online. */
const mailboxLabels: Record<string, string> = {
  notConfigured: '离线信箱未配置',
  connecting: '正在连接离线信箱',
  online: '离线信箱可用',
  retrying: '离线信箱连接中断',
  error: '离线信箱不可用',
  paused: '离线信箱已暂停',
};

function renderMailbox() {
  const mailbox = state.mailbox;
  const queued = mailbox.pending > 0 ? ` · ${mailbox.pending} 条待发` : '';
  const label = `${mailboxLabels[mailbox.state] ?? '离线信箱状态未知'}${queued}`;
  const tone = mailbox.state === 'online' ? 'online' : mailbox.state === 'error' ? 'error' : mailbox.state === 'retrying' ? 'retrying' : 'muted';

  const block = $('.mailbox');
  block.setAttribute('data-state', mailbox.state);
  block.dataset.tone = tone;
  block.title = mailbox.message ?? '';
  $('.mailbox-text').textContent = label;

  const pill = $('.mailbox-pill');
  pill.setAttribute('data-state', mailbox.state);
  pill.dataset.tone = tone;
  pill.title = mailbox.message
    ? `离线信箱：${mailbox.message}`
    : `离线信箱：${mailboxLabels[mailbox.state] ?? '状态未知'}`;
  $('.mailbox-pill-text').textContent = mailbox.state === 'notConfigured' ? '信箱未配置' : mailbox.state === 'online' ? '信箱可用' : '信箱未就绪';

  const roomDialog = $<HTMLDialogElement>('#dlg-room');
  if (roomDialog.open) {
    $('#room-mailbox').textContent = `${label}`;
  }
}

function renderComposer() {
  const connected = state.connection === 'connected';
  const hasRoom = !!state.room;
  const input = $<HTMLTextAreaElement>('#message-input');
  const peer = state.peerName ?? '对方';

  /* With a saved room the mailbox outbox accepts messages offline; without one
   * there is nowhere to send them. The draft is never rewritten here. */
  input.disabled = !hasRoom;
  input.placeholder = connected
    ? `发消息给 ${peer}…`
    : hasRoom
      ? '对方不在线，消息会先加密暂存，上线后送达'
      : '配对后可以发送消息';

  $<HTMLButtonElement>('.send-button').disabled = !hasRoom || sending || !input.value.trim();
  $<HTMLButtonElement>('.attach-button').disabled = !hasRoom || preparing;
  $<HTMLButtonElement>('.choose-file').disabled = !hasRoom || preparing;
  $('.composer-hint').textContent = connected
    ? 'Enter 发送，Shift + Enter 换行'
    : hasRoom
      ? '离线也能发送：消息会加密暂存到服务器'
      : '配对后才能发送消息';
}

function renderServerSettings() {
  $('#server-config').hidden = !state.server;
  $('#server-none').hidden = !!state.server;
  if (state.server) {
    $('#server-relay').textContent = state.server.relayUrl;
    $('#server-mailbox').textContent = state.server.mailboxHost;
  }
}

function renderRoomDialog() {
  const room = state.room;
  $('.room-key-block').hidden = !(room && room.role === 'host');
  $('.forget-room').textContent = '忘记这个房间';
  $('.forget-room').dataset.confirm = 'false';
  $('.forget-hint').hidden = true;
  $<HTMLTextAreaElement>('.room-key').hidden = true;
  $<HTMLTextAreaElement>('.room-key').value = '';
  const error = $('#dlg-room .form-error');
  error.textContent = '';
  if (!room) return;
  $('#room-role').textContent = room.role === 'host' ? '房主（创建房间）' : '加入方';
  $('#room-id').textContent = room.roomId;
  $('#room-network').textContent = room.networkName;
  $('#room-ip').textContent = room.virtualIp;
  $('#room-peer-ip').textContent = room.peerVirtualIp;
  $('#room-route').textContent =
    state.connection === 'connected' && state.route === 'direct' ? 'P2P 直连' : '尚无已验证的直连';
  $('#room-server').textContent = state.server
    ? `${state.server.relayUrl} · 信箱 ${state.server.mailboxHost}`
    : '尚未导入服务器设置码';
  $('#room-mailbox').textContent = `${mailboxLabels[state.mailbox.state] ?? '状态未知'}${
    state.mailbox.pending > 0 ? `（${state.mailbox.pending} 条待发）` : ''
  }`;
}

function renderAll() {
  syncRoomWindow();
  renderChrome();
  renderThread();
  renderTransfers();
  renderServerSettings();
}

/* ---------- data ---------- */

function mergeMessage(message: Message) {
  const index = state.messages.findIndex(item => item.id === message.id);
  /* A delivery/via update for a known ID replaces it in place; it must never be
   * ignored, and it must not be re-appended. An event for a message outside the
   * loaded window (an old receipt) keeps chronological order instead of being
   * shown as the newest row. */
  if (index >= 0) {
    state.messages[index] = message;
  } else {
    const at = state.messages.findIndex(item => item.timestamp > message.timestamp);
    if (at < 0) state.messages.push(message);
    else state.messages.splice(at, 0, message);
  }
  /* Keep loaded history; only bound memory against pathological growth. */
  if (state.messages.length > 5000) state.messages = state.messages.slice(-5000);
}

function mergeTransfer(transfer: Transfer) {
  const index = state.transfers.findIndex(item => item.id === transfer.id);
  if (index < 0) state.transfers.push(transfer);
  else state.transfers[index] = transfer;
}

async function refresh() {
  try {
    adoptSnapshot(await api.snapshot());
  } catch (error) {
    toast(`无法读取当前状态：${String(error)}`);
  }
}

/** Replace app state, but keep already-loaded older history for the same room.
 * A resync or refresh must not silently truncate the transcript to the newest
 * page, nor make the "load earlier" control disappear. */
function adoptSnapshot(snapshot: Snapshot) {
  const sameRoom = (snapshot.room?.roomId ?? null) === viewRoomId;
  const carried = sameRoom ? state.messages : [];
  state = snapshot;
  if (sameRoom) state.messages = mergeMessages(carried, snapshot.messages);
  renderAll();
}

/** Merge two chronological lists by id; `fresh` wins, order stays oldest-first. */
function mergeMessages(carried: Message[], fresh: Message[]): Message[] {
  if (!carried.length) return fresh;
  const latest = new Map(fresh.map(message => [message.id, message]));
  const merged: Message[] = [];
  const seen = new Set<string>();
  for (const message of carried) {
    merged.push(latest.get(message.id) ?? message);
    seen.add(message.id);
  }
  for (const message of fresh) {
    if (seen.has(message.id)) continue;
    seen.add(message.id);
    merged.push(message);
  }
  return merged;
}

function applyEvent(event: AppEvent) {
  if (!ready) {
    pending.push(event);
    return;
  }
  if (event.type === 'state') {
    const previousRoom = state.room?.roomId ?? null;
    state.connection = event.connection;
    state.route = event.connection === 'connected' ? 'direct' : 'unknown';
    state.peerName = event.peerName;
    state.paused = event.paused;
    state.reconnect = event.reconnect;
    state.room = event.room;
    /* a different room means a different transcript; reload it from disk */
    if ((event.room?.roomId ?? null) !== previousRoom) void refresh();
    /* re-render everything: transfer actions depend on the connection state */
    renderAll();
  } else if (event.type === 'server') {
    state.server = event.server;
    renderServerSettings();
    renderChrome();
  } else if (event.type === 'message') {
    mergeMessage(event.message);
    renderThread();
  } else if (event.type === 'transfer') {
    const isNew = !state.transfers.some(item => item.id === event.transfer.id);
    mergeTransfer(event.transfer);
    renderTransfers();
    if (isNew && !event.transfer.outgoing && event.transfer.status === 'offered') {
      toast(state.profile.autoAcceptFiles
        ? `收到文件请求：${event.transfer.name}，已按设置自动接收。`
        : `收到文件请求：${event.transfer.name}，请在文件传输中确认。`);
    }
  } else if (event.type === 'mailbox') {
    state.mailbox = { state: event.state, message: event.message, pending: event.pending };
    renderMailbox();
  } else {
    toast(event.message);
  }
}

/* ---------- dialogs ---------- */

function setPairMode(mode: 'create' | 'join') {
  const radio = $(`#pair-form [name="mode"][value="${mode}"]`) as HTMLInputElement;
  radio.checked = true;
  $('.pair-create').hidden = mode !== 'create';
  $('.pair-join').hidden = mode !== 'join';
  $('.pair-submit').hidden = mode !== 'join';
  const error = $('#dlg-pair .form-error');
  error.textContent = '';
  if (mode === 'join') $<HTMLTextAreaElement>('#pair-form [name="invite"]').focus();
}

function openPair(mode: 'create' | 'join' = 'create') {
  const dialog = $<HTMLDialogElement>('#dlg-pair');
  $('.pair-choose').hidden = false;
  $('.pair-foot').hidden = false;
  $('.pair-key-panel').hidden = true;
  $<HTMLTextAreaElement>('.pairing-code').value = '';
  $<HTMLTextAreaElement>('#pair-form [name="invite"]').value = '';
  setPairMode(mode);
  $('.pair-server-hint').hidden = state.server !== null;
  if (!dialog.open) dialog.showModal();
  if (mode === 'create') $<HTMLButtonElement>('.create-room').focus();
}

function openRoom() {
  if (!state.room) return;
  const dialog = $<HTMLDialogElement>('#dlg-room');
  renderRoomDialog();
  if (!dialog.open) dialog.showModal();
  $<HTMLButtonElement>('#dlg-room .dialog-close').focus();
}

function openSettings() {
  $<HTMLInputElement>('#settings-form [name="displayName"]').value = state.profile.displayName;
  $<HTMLInputElement>('#settings-form [name="downloadDir"]').value = state.profile.downloadDir;
  $<HTMLInputElement>('#settings-form [name="autoAcceptFiles"]').checked = state.profile.autoAcceptFiles ?? false;
  resetImport();
  $<HTMLTextAreaElement>('#server-code').value = '';
  $('#server-error').textContent = '';
  renderServerSettings();
  const dialog = $<HTMLDialogElement>('#dlg-settings');
  if (!dialog.open) dialog.showModal();
  $<HTMLInputElement>('#settings-form [name="displayName"]').focus();
}

function setPairBusy(busy: boolean) {
  pairBusy = busy;
  const dialog = $<HTMLDialogElement>('#dlg-pair');
  dialog.dataset.busy = busy ? 'true' : 'false';
  $$('#dlg-pair .dialog-close, #dlg-pair .dialog-foot .btn.quiet, #dlg-pair .create-room, #dlg-pair .copy-key, #dlg-pair .done-pair, #dlg-pair .pair-submit').forEach(
    button => ((button as HTMLButtonElement).disabled = busy),
  );
}

async function copyText(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    /* Clipboard API may be unavailable (permissions, insecure context). */
  }
  const helper = document.createElement('textarea');
  helper.value = text;
  helper.setAttribute('readonly', '');
  helper.style.position = 'fixed';
  helper.style.opacity = '0';
  document.body.append(helper);
  helper.select();
  let copied = false;
  try {
    copied = document.execCommand('copy');
  } catch {
    copied = false;
  }
  helper.remove();
  return copied;
}

/* ---------- appearance & theme plugins ---------- */

const modeLabels: Record<ThemeMode, string> = { light: '浅色', dark: '深色', system: '跟随系统' };

let pendingConfirm: (() => void) | null = null;

function themeCardMarkup(theme: ThemeSummary): string {
  const palette = paletteFor(getAppearance().resolved, getTheme(theme.id) ?? undefined);
  const preview = theme.builtin ? palette : { ...palette, accent: theme[getAppearance().resolved].accent };
  const remove = theme.builtin
    ? ''
    : `<button type="button" class="theme-remove" data-remove-id="${esc(theme.id)}" aria-label="删除主题 ${esc(theme.name)}" title="删除主题">${icon('trash')}</button>`;
  return `<div class="theme-option">
    <button type="button" class="theme-card" role="radio" data-theme-id="${esc(theme.id)}" aria-checked="false">
      <span class="theme-preview" style="--pv-canvas:${esc(preview.canvas)};--pv-sidebar:${esc(preview.sidebar)};--pv-ink:${esc(preview.ink)};--pv-line:${esc(preview.line)};--pv-accent:${esc(preview.accent)}" aria-hidden="true">
        <i class="pv-bar"></i><i class="pv-panel"></i><i class="pv-dot"></i><i class="pv-line"></i><i class="pv-line short"></i>
      </span>
      <span class="theme-meta">
        <span class="theme-name">${esc(theme.name)}</span>
        <span class="theme-sub">${theme.builtin ? '内置' : `已安装 · v${esc(theme.version)}`}</span>
      </span>
    </button>
    ${remove}
  </div>`;
}

function setText(id: string, text: string) {
  const node = document.getElementById(id);
  if (node) node.textContent = text;
}

function renderAppearance() {
  const appearance = getAppearance();
  const grid = document.getElementById('theme-grid');
  if (grid) grid.innerHTML = listThemes().map(themeCardMarkup).join('');
  $$('#theme-grid .theme-card').forEach(card => {
    const active = card.dataset.themeId === appearance.themeId;
    card.setAttribute('aria-checked', active ? 'true' : 'false');
    card.classList.toggle('active', active);
  });
  $$('#dlg-appearance .segmented-item').forEach(item => {
    const active = item.dataset.mode === appearance.mode;
    item.classList.toggle('active', active);
    item.setAttribute('aria-pressed', active ? 'true' : 'false');
  });
  const active = getTheme(appearance.themeId);
  setText(
    'appearance-mode-hint',
    appearance.mode === 'system'
      ? `当前跟随系统，显示为${modeLabels[appearance.resolved]}。`
      : `固定为${modeLabels[appearance.mode]}，不随系统切换。`,
  );
  const accent = document.getElementById('accent-input') as HTMLInputElement | null;
  if (accent) accent.value = appearance.accent ?? (active ? active[appearance.resolved].accent : '#3b6ee0');
  const size = document.getElementById('font-size-input') as HTMLInputElement | null;
  const effectiveSize = appearance.fontSize ?? active?.style.fontSize ?? 14;
  if (size) size.value = String(effectiveSize);
  setText('font-size-value', `${effectiveSize}px`);
}

function setThemeError(message: string) {
  setText('theme-error', message);
}

function applyThemePack(raw: string, replace = false) {
  const result = installTheme(raw, { replace });
  if (result.ok) {
    setThemeError('');
    setText('theme-plugin-status', `已${result.replaced ? '更新' : '安装'}主题「${result.theme.name}」。`);
    setTheme(result.theme.id);
    return;
  }
  if (result.conflict) {
    askConfirm(
      `已安装同名主题「${result.conflict.name}」（v${result.conflict.version}）。覆盖后原主题会被替换，主题 ID 不变。`,
      '覆盖主题',
      () => applyThemePack(raw, true),
    );
    return;
  }
  setThemeError(result.error);
}

function downloadTheme(json: string) {
  const blob = new Blob([json], { type: 'application/json' });
  const url = URL.createObjectURL(blob);
  const link = document.createElement('a');
  link.href = url;
  link.download = 'twinlink-theme.json';
  document.body.append(link);
  link.click();
  link.remove();
  setTimeout(() => URL.revokeObjectURL(url), 4000);
}

function openAppearance() {
  setThemeError('');
  clearConfirm();
  setText('theme-plugin-status', '');
  renderAppearance();
  const dialog = $<HTMLDialogElement>('#dlg-appearance');
  if (!dialog.open) dialog.showModal();
}

function confirmRemoveTheme(id: string) {
  const theme = getTheme(id);
  if (!theme) return;
  const wasActive = getAppearance().themeId === id;
  askConfirm(
    wasActive
      ? `删除主题「${theme.name}」？它是当前主题，删除后会回到默认主题。`
      : `删除主题「${theme.name}」？删除后需要重新导入主题包才能恢复。`,
    '删除主题',
    () => {
      const result = uninstallTheme(id);
      if (!result.ok) {
        setThemeError(result.error);
        return;
      }
      setThemeError('');
      setText('theme-plugin-status', result.fellBack ? '主题已删除，已回到默认主题。' : '主题已删除。');
    },
  );
}

/** Confirmation lives inside the dialog so no WebView script dialog is involved. */
function askConfirm(message: string, actionLabel: string, run: () => void) {
  pendingConfirm = run;
  setText('theme-confirm-message', message);
  setText('theme-confirm-accept', actionLabel);
  const box = document.getElementById('theme-confirm');
  if (box) box.hidden = false;
  document.getElementById('theme-confirm-accept')?.focus();
}

function clearConfirm() {
  pendingConfirm = null;
  const box = document.getElementById('theme-confirm');
  if (box) box.hidden = true;
  setText('theme-confirm-message', '');
}

$('#dlg-appearance').addEventListener('click', event => {
  const target = event.target as HTMLElement;
  if (target.closest('.dialog-close')) {
    $<HTMLDialogElement>('#dlg-appearance').close();
    return;
  }
  const remove = target.closest<HTMLElement>('.theme-remove');
  if (remove?.dataset.removeId) {
    event.stopPropagation();
    confirmRemoveTheme(remove.dataset.removeId);
    return;
  }
  const card = target.closest<HTMLElement>('.theme-card');
  if (card?.dataset.themeId) {
    if (setTheme(card.dataset.themeId)) {
      setThemeError('');
      setText('theme-plugin-status', `已切换到「${getTheme(card.dataset.themeId)?.name ?? ''}」。`);
    }
    return;
  }
  const mode = target.closest<HTMLElement>('.segmented-item')?.dataset.mode;
  if (mode === 'light' || mode === 'dark' || mode === 'system') setMode(mode);
});

$('#dlg-appearance').addEventListener('keydown', event => {
  if (event.key !== 'Escape') return;
  if (pendingConfirm) {
    event.preventDefault();
    clearConfirm();
    setText('theme-plugin-status', '已取消，没有改动已安装的主题。');
  }
});

$('#accent-input').addEventListener('input', event => {
  setAccent((event.target as HTMLInputElement).value);
});
$('#accent-clear').addEventListener('click', () => setAccent(null));
$('#font-size-input').addEventListener('input', event => {
  setFontSize(Number((event.target as HTMLInputElement).value));
});
$('#theme-confirm-accept').addEventListener('click', () => {
  const run = pendingConfirm;
  clearConfirm();
  run?.();
});

$('#theme-confirm-cancel').addEventListener('click', () => {
  clearConfirm();
  setText('theme-plugin-status', '已取消，没有改动已安装的主题。');
});

$('#dlg-appearance').addEventListener('close', () => clearConfirm());

$('#theme-reset').addEventListener('click', () => {
  resetAppearance();
  setThemeError('');
  setText('theme-plugin-status', '已恢复默认外观：默认主题、跟随系统、无自定义强调色。');
  const text = document.getElementById('theme-json') as HTMLTextAreaElement | null;
  if (text) text.value = exportThemeJson();
});
$('#theme-export').addEventListener('click', () => {
  const json = exportThemeJson();
  downloadTheme(json);
  const text = document.getElementById('theme-json') as HTMLTextAreaElement | null;
  if (text) text.value = json;
  setText('theme-plugin-status', '已生成主题包文件 twinlink-theme.json；如浏览器未出现下载，可复制下方 JSON 自行保存。');
});
$('#theme-copy').addEventListener('click', async () => {
  const text = document.getElementById('theme-json') as HTMLTextAreaElement | null;
  const json = text?.value.trim() || exportThemeJson();
  const copied = await copyText(json);
  setText('theme-plugin-status', copied ? '主题 JSON 已复制到剪贴板。' : '无法访问剪贴板，请手动选中下方 JSON 复制。');
});
$('#theme-import').addEventListener('click', () => {
  const file = document.getElementById('theme-file') as HTMLInputElement | null;
  if (file) file.click();
});
$('#theme-apply').addEventListener('click', () => {
  const text = document.getElementById('theme-json') as HTMLTextAreaElement | null;
  const raw = text?.value.trim() ?? '';
  if (!raw) {
    setThemeError('文本框是空的，请先粘贴主题包 JSON，或改用“导入主题包文件”。');
    return;
  }
  setThemeError('');
  applyThemePack(raw);
});

$('#theme-file').addEventListener('change', async event => {
  const input = event.target as HTMLInputElement;
  const file = input.files?.[0];
  if (!file) return;
  if (file.size > MAX_THEME_BYTES) {
    setThemeError('主题包超过 64 KiB，请确认这是主题 JSON 文件。');
    input.value = '';
    return;
  }
  try {
    const raw = await file.text();
    const text = document.getElementById('theme-json') as HTMLTextAreaElement | null;
    if (text) text.value = raw;
    applyThemePack(raw);
  } catch {
    setThemeError('无法读取该文件，请重新选择主题 JSON 文件。');
  } finally {
    input.value = '';
  }
});

/* The theme runtime owns all state; this only mirrors it into the dialog. */
subscribeTheme(() => renderAppearance());

/* ---------- wiring ---------- */


$$('[data-page]').forEach(button =>
  button.addEventListener('click', () => showPage((button as HTMLElement).dataset.page === 'files' ? 'files' : 'chat'))
);
$$('.show-files').forEach(button => button.addEventListener('click', () => showPage('files')));
$$('.settings-open').forEach(button => button.addEventListener('click', openSettings));
$$('.appearance-open').forEach(button => button.addEventListener('click', openAppearance));
$$('.open-appearance').forEach(button =>
  button.addEventListener('click', () => {
    $<HTMLDialogElement>('#dlg-settings').close();
    openAppearance();
  })
);
$$('.settings-trigger').forEach(button => button.addEventListener('click', openSettings));
$$('.room-open').forEach(button => button.addEventListener('click', openRoom));

/* the empty-state actions and inline banners are re-rendered, so delegate them */
document.addEventListener('click', event => {
  const target = event.target as HTMLElement | null;
  if (target?.closest('.open-pair')) openPair('create');
  else if (target?.closest('.open-room')) openRoom();
  else if (target?.closest('.resume-action')) void resume();
  else if (target?.closest('.open-settings-from-pair')) {
    $<HTMLDialogElement>('#dlg-pair').close();
    openSettings();
  }
});

async function pause() {
  try {
    await api.pause();
    await refresh();
  } catch (error) {
    toast(String(error));
  }
}

async function resume() {
  if (!state.room) {
    openPair('create');
    return;
  }
  try {
    await api.resume();
    await refresh();
    toast('正在重新连接已保存的房间。');
  } catch (error) {
    toast(String(error));
  }
}

$<HTMLButtonElement>('.conn-action').addEventListener('click', async () => {
  if (state.room && (state.paused || state.connection === 'disconnected')) {
    await resume();
    return;
  }
  if (state.connection !== 'disconnected') {
    await pause();
    return;
  }
  openPair('create');
});

$$('.dialog-close').forEach(button =>
  button.addEventListener('click', () => (button as HTMLElement).closest('dialog')?.close())
);

$$<HTMLDialogElement>('dialog').forEach(dialog => {
  dialog.addEventListener('close', () => {
    const error = dialog.querySelector('.form-error');
    if (error) error.textContent = '';
    /* pairing keys and server setup codes are credentials: clear them on close */
    dialog.querySelectorAll<HTMLTextAreaElement>('.pairing-code, .room-key, [name="invite"]').forEach(field => {
      field.value = '';
    });
    /* only the intentionally collapsed outputs stay hidden; the join input must
       be visible again the next time the pairing dialog opens */
    dialog.querySelectorAll<HTMLTextAreaElement>('.pairing-code, .room-key').forEach(field => {
      field.hidden = true;
    });
    $<HTMLTextAreaElement>('#pair-form [name="invite"]').hidden = false;
    const serverCode = dialog.querySelector<HTMLTextAreaElement>('#server-code');
    if (serverCode) serverCode.value = '';
    if (dialog.id === 'dlg-pair') {
      $('.pair-choose').hidden = false;
      $('.pair-foot').hidden = false;
      $('.pair-key-panel').hidden = true;
      setPairBusy(false);
    }
    if (dialog.id === 'dlg-room') renderRoomDialog();
  });
  /* close on a real backdrop click only: clicks inside the panel (padding, scrollbar,
     padding around fields) keep the dialog and the typed credential */
  dialog.addEventListener('click', event => {
    if (dialog.dataset.busy === 'true') return;
    const rect = dialog.getBoundingClientRect();
    const outside =
      event.clientX < rect.left ||
      event.clientX > rect.right ||
      event.clientY < rect.top ||
      event.clientY > rect.bottom;
    if (outside) dialog.close();
  });
});

$('#pair-form').addEventListener('change', event => {
  const target = event.target as HTMLInputElement;
  if (target.name !== 'mode') return;
  setPairMode(target.value === 'join' ? 'join' : 'create');
});

$('.create-room').addEventListener('click', async () => {
  const error = $('#dlg-pair .form-error');
  error.textContent = '';
  if (!state.server) {
    error.textContent = '请先在设置中导入服务器设置码。';
    $('.pair-server-hint').hidden = false;
    return;
  }
  setPairBusy(true);
  error.textContent = '';
  try {
    const key = await api.createRoom();
    /* show the key immediately: even if the follow-up snapshot fails, the user
       must not lose the one-time pairing credential */
    $<HTMLTextAreaElement>('.pairing-code').value = key;
    $('.pair-choose').hidden = true;
    $('.pair-foot').hidden = true;
    $('.pair-key-panel').hidden = false;
    $<HTMLButtonElement>('.pair-key-panel .copy-key').focus();
    try {
      state = await api.snapshot();
      renderAll();
    } catch {
      toast('房间已创建，但界面状态刷新失败；请复制配对码后再重新打开应用。');
    }
  } catch (cause) {
    error.textContent = String(cause);
    error.scrollIntoView({ block: 'nearest' });
  } finally {
    setPairBusy(false);
  }
});

$$('.copy-key').forEach(button => button.addEventListener('click', async event => {
  const trigger = event.currentTarget as HTMLButtonElement;
  const panel = trigger.closest('.pair-key-panel') as HTMLElement | null;
  const roomKey = trigger.closest('.room-key-block') as HTMLElement | null;
  let text = '';
  try {
    if (panel) text = $<HTMLTextAreaElement>('.pairing-code').value;
    else if (roomKey) {
      text = await api.pairingKey();
      $<HTMLTextAreaElement>('.room-key').value = text;
    }
  } catch (error) {
    toast(String(error));
    return;
  }
  if (!text) {
    toast('当前没有可复制的配对码。');
    return;
  }
  if (await copyText(text)) {
    toast('配对码已复制。请只发给你信任的人。');
    if (roomKey) $<HTMLTextAreaElement>('.room-key').hidden = true;
  } else {
    if (roomKey) {
      $<HTMLTextAreaElement>('.room-key').hidden = false;
    }
    toast('无法自动复制。配对码已显示，请手动全选复制。');
  }
}));

$('.done-pair').addEventListener('click', () => $<HTMLDialogElement>('#dlg-pair').close());

$<HTMLFormElement>('#pair-form').addEventListener('submit', async event => {
  event.preventDefault();
  if (pairBusy) return;
  const form = event.currentTarget as HTMLFormElement;
  const error = form.querySelector('.form-error')!;
  const submit = form.querySelector<HTMLButtonElement>('.pair-submit')!;
  error.textContent = '';

  const code = $<HTMLTextAreaElement>('#pair-form [name="invite"]').value.trim();
  if (!code) {
    error.textContent = '请粘贴对方给你的配对码。';
    $<HTMLTextAreaElement>('#pair-form [name="invite"]').focus();
    return;
  }
  if (!code.startsWith('TL1.') || code.length > 16 * 1024) {
    error.textContent = '这不是有效的配对码。请确认以 TL1. 开头，且复制完整。';
    $<HTMLTextAreaElement>('#pair-form [name="invite"]').focus();
    return;
  }

  setPairBusy(true);
  submit.disabled = true;
  try {
    await api.joinRoom(code);
    $<HTMLDialogElement>('#dlg-pair').close();
    await refresh();
    toast('已加入房间，正在连接对方。');
  } catch (cause) {
    error.textContent = String(cause);
    error.scrollIntoView({ block: 'nearest' });
  } finally {
    setPairBusy(false);
    submit.disabled = false;
  }
});

$('#dlg-room .forget-room').addEventListener('click', async event => {
  const button = event.currentTarget as HTMLButtonElement;
  const hint = $('.forget-hint');
  if (button.dataset.confirm !== 'true') {
    button.dataset.confirm = 'true';
    button.textContent = '确认忘记房间？';
    hint.hidden = false;
    return;
  }
  const error = $('#dlg-room .form-error');
  error.textContent = '';
  button.disabled = true;
  try {
    await api.forgetRoom();
    $<HTMLDialogElement>('#dlg-room').close();
    await refresh();
    toast('已忘记房间。聊天记录仍保留在本机。');
  } catch (cause) {
    error.textContent = String(cause);
  } finally {
    button.disabled = false;
  }
});

$('.import-server').addEventListener('click', async event => {
  const button = event.currentTarget as HTMLButtonElement;
  const input = $<HTMLTextAreaElement>('#server-code');
  const error = $('#server-error');
  const code = input.value.trim();
  error.textContent = '';
  if (!code) {
    error.textContent = '请粘贴服务器设置码。';
    input.focus();
    return;
  }
  if (!code.startsWith('TL-SRV1.')) {
    error.textContent = '这不是有效的服务器设置码。请确认以 TL-SRV1. 开头。';
    input.focus();
    return;
  }
  if (code.length > 32 * 1024) {
    error.textContent = '服务器设置码过长，请确认复制内容完整。';
    return;
  }
  button.disabled = true;
  try {
    await api.importServer(code);
    input.value = '';
    state = await api.snapshot();
    renderServerSettings();
    renderChrome();
    toast('服务器设置码已导入。');
  } catch (cause) {
    error.textContent = String(cause);
  } finally {
    button.disabled = false;
  }
});

$('.choose-directory').addEventListener('click', async () => {
  try {
    const path = await api.chooseFolder();
    if (path) $<HTMLInputElement>('#settings-form [name="downloadDir"]').value = path;
  } catch (error) {
    $('#settings-form .form-error').textContent = String(error);
  }
});

$<HTMLFormElement>('#settings-form').addEventListener('submit', async event => {
  event.preventDefault();
  const form = event.currentTarget as HTMLFormElement;
  const error = form.querySelector('.form-error')!;
  const name = form.querySelector<HTMLInputElement>('[name="displayName"]')!;
  const profile = {
    displayName: name.value.trim(),
    downloadDir: form.querySelector<HTMLInputElement>('[name="downloadDir"]')!.value,
    autoAcceptFiles: form.querySelector<HTMLInputElement>('[name="autoAcceptFiles"]')!.checked,
  };
  error.textContent = '';
  if (!profile.displayName) {
    error.textContent = '请填写设备名称。';
    name.focus();
    return;
  }
  if (!profile.downloadDir) {
    error.textContent = '请选择文件保存位置。';
    return;
  }
  const submit = form.querySelector<HTMLButtonElement>('[type="submit"]')!;
  submit.disabled = true;
  try {
    await api.saveProfile(profile);
    state.profile = profile;
    renderChrome();
    $<HTMLDialogElement>('#dlg-settings').close();
    toast('设置已保存。');
  } catch (cause) {
    error.textContent = String(cause);
  } finally {
    submit.disabled = false;
  }
});

/* ---------- local backups ---------- */
let pendingImport: string | null = null;
let backupBusy = false;
function resetImport() {
  pendingImport = null;
  $('#import-data').textContent = '选择备份导入';
  $('#cancel-import').hidden = true;
  $('#backup-status').textContent = '';
}
$('#cancel-import').addEventListener('click', resetImport);
$('#export-data').addEventListener('click', async () => {
  if (backupBusy) return;
  backupBusy = true;
  try {
    const path = await api.chooseExport();
    if (!path) return;
    $('#backup-status').textContent = '正在导出备份…';
    await api.exportData(path);
    $('#backup-status').textContent = '备份已保存，包含房间凭据，请妥善保管。';
  } catch (error) { $('#backup-status').textContent = String(error); }
  finally { backupBusy = false; }
});
$('#import-data').addEventListener('click', async () => {
  if (backupBusy) return;
  backupBusy = true;
  try {
    if (!pendingImport) {
      pendingImport = await api.chooseBackup();
      if (!pendingImport) return;
      $('#import-data').textContent = '确认导入并暂停连接';
      $('#cancel-import').hidden = false;
      $('#backup-status').textContent = '已选择备份。确认后合并记录，不覆盖现有记录或附件；连接将暂停，需手动恢复。';
      return;
    }
    if (state.room) await api.pause();
    const report = await api.importData(pendingImport);
    pendingImport = null;
    adoptSnapshot(await api.snapshot());
    openSettings();
    $('#backup-status').textContent = `导入完成：新增 ${report.messagesAdded} 条聊天、${report.transfersAdded} 条文件记录。自动接收已关闭；需要联机时请手动恢复连接。`;
  } catch (error) {
    $('#backup-status').textContent = String(error);
    try { adoptSnapshot(await api.snapshot()); } catch { /* Keep the original import error visible. */ }
  } finally { backupBusy = false; }
});

/* ---------- messaging ---------- */

const input = $<HTMLTextAreaElement>('#message-input');

function resizeInput() {
  input.style.height = 'auto';
  input.style.height = `${Math.min(input.scrollHeight, 160)}px`;
}

async function send() {
  const text = input.value;
  if (sending || !text.trim() || !state.room) return;
  sending = true;
  renderComposer();
  try {
    mergeMessage(await api.sendMessage(text));
    renderThread();
    if (input.value === text) {
      input.value = '';
      resizeInput();
    }
  } catch (error) {
    toast(String(error));
  } finally {
    sending = false;
    renderComposer();
    input.focus();
  }
}

$<HTMLFormElement>('#composer').addEventListener('submit', event => {
  event.preventDefault();
  void send();
});

input.addEventListener('input', () => {
  renderComposer();
  resizeInput();
});

input.addEventListener('keydown', event => {
  const composing = event.isComposing || event.keyCode === 229;
  if (event.key === 'Enter' && !event.shiftKey && !composing) {
    event.preventDefault();
    void send();
  }
});

/* ---------- files ---------- */

async function sendSelectedFiles(paths: string[]) {
  if (preparing) { toast('正在处理已选择的文件，请稍后再拖入。'); return; }
  if (!state.room || state.preview) { toast('请先在桌面应用中创建或加入房间。'); return; }
  preparing = true;
  renderComposer();
  try {
    for (const path of [...new Set(paths)]) {
      try {
        toast('正在读取所选文件并计算校验值…');
        const transfer = await api.offerFile(path);
        mergeTransfer(transfer);
        renderTransfers();
        showPage('files');
        toast(transfer.via === 'direct'
          ? '已发送 P2P 文件邀请，等待对方接收。'
          : '无法直连，文件已加入加密离线暂存队列。');
      } catch (error) { toast(String(error)); }
    }
  } finally {
    preparing = false;
    renderComposer();
  }
}

async function chooseFile() {
  if (preparing) return;
  try {
    const path = await api.chooseFile();
    if (path) await sendSelectedFiles([path]);
  } catch (error) { toast(String(error)); }
}

$('.attach-button').addEventListener('click', chooseFile);
$('.choose-file').addEventListener('click', chooseFile);
// Browser File objects are never converted to guessed paths or fake uploads.
document.addEventListener('dragover', event => event.preventDefault());
document.addEventListener('drop', event => {
  event.preventDefault();
  if (state.preview) toast('拖拽发送文件需要使用桌面应用。');
});

$('.transfers').addEventListener('click', async event => {
  const button = (event.target as HTMLElement).closest<HTMLButtonElement>('button');
  if (!button) return;
  button.disabled = true;
  try {
    if (button.dataset.accept) await api.acceptFile(button.dataset.accept);
    else if (button.dataset.reject) await api.rejectFile(button.dataset.reject);
    else if (button.dataset.cancel) await api.cancelTransfer(button.dataset.cancel);
  } catch (error) {
    toast(String(error));
    button.disabled = false;
  }
});

$('.history-more').addEventListener('click', () => void loadEarlier());

/* ---------- boot ---------- */

async function initialize() {
  try {
    /* subscribe before snapshot so no event is lost */
    await api.subscribe(applyEvent);
    await api.onResync(snapshot => adoptSnapshot(snapshot));
    state = await api.snapshot();
    await api.onFileDrop(event => {
      const allowed = !!state.room && !state.preview && !document.querySelector('dialog[open]');
      $('#file-drop-hint').hidden = !allowed || (event.type !== 'enter' && event.type !== 'over');
      if (event.type === 'drop') {
        $('#file-drop-hint').hidden = true;
        if (allowed) void sendSelectedFiles(event.paths);
        else if (!document.querySelector('dialog[open]')) toast('请先创建或加入房间再拖入文件。');
      }
    });
    ready = true;
    for (const event of pending.splice(0)) applyEvent(event);
    renderAll();
    resizeInput();
  } catch (error) {
    ready = true;
    renderAll();
    toast(`初始化失败：${String(error)}`);
  }
}

initializeUpdates(toast);
void initialize();
