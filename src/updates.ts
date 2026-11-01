import packageInfo from '../package.json';
import { api, type UpdateInfo } from './api';

export const versionLabel = `v${packageInfo.version} · 预览版`;

/** Checks may run in the background. Installation only follows an explicit click. */
export function initializeUpdates(notify: (text: string) => void) {
  const check = document.querySelector<HTMLButtonElement>('#check-update')!;
  const install = document.querySelector<HTMLButtonElement>('#install-update')!;
  const status = document.querySelector<HTMLElement>('#update-status')!;
  const notes = document.querySelector<HTMLElement>('#update-notes')!;
  const progress = document.querySelector<HTMLProgressElement>('#update-progress')!;
  let available: UpdateInfo | null = null;
  let busy = false;
  let installing = false;

  async function checkNow(automatic = false) {
    if (busy) return;
    busy = true;
    available = null;
    check.disabled = true;
    install.hidden = true;
    notes.hidden = true;
    progress.hidden = true;
    status.textContent = '正在连接 GitHub 检查更新…';
    try {
      const result = await api.checkUpdate();
      if (result.version) {
        available = result;
        install.textContent = `更新至 v${result.version} 并重启`;
        install.hidden = false;
        status.textContent = `发现新版本 v${result.version}。安装后会重启，聊天记录会保留，未完成的传输在重启后恢复。`;
        notes.textContent = result.notes || '';
        notes.hidden = !result.notes;
        if (automatic) notify(`发现同频新版本 v${result.version}，可在设置中更新。`);
      } else {
        status.textContent = '当前已是最新预览版。更新源：GitHub。';
      }
    } catch (error) {
      status.textContent = String(error);
    } finally {
      busy = false;
      check.disabled = false;
    }
  }

  check.addEventListener('click', () => void checkNow());
  install.addEventListener('click', async () => {
    if (busy || !available?.version) return;
    busy = true;
    installing = true;
    check.disabled = true;
    install.disabled = true;
    progress.hidden = false;
    progress.removeAttribute('value');
    status.textContent = '正在从 GitHub 下载并校验更新签名，请保持应用开启…';
    try {
      await api.installUpdate();
    } catch (error) {
      installing = false;
      busy = false;
      status.textContent = String(error);
      progress.hidden = true;
      check.disabled = false;
      install.disabled = false;
    }
  });
  if (!api.isDesktop()) {
    check.disabled = true;
    status.textContent = '界面预览不检查或安装更新，请在桌面应用中使用。';
    return;
  }
  void api.onUpdateProgress(event => {
    if (!installing) return;
    if (event.phase === 'installing') {
      status.textContent = '签名校验通过，正在安装并重启…';
      progress.value = 1;
    } else if (event.total && event.total > 0) {
      progress.value = Math.min(1, event.downloaded / event.total);
      status.textContent = `正在下载更新 ${Math.floor(progress.value * 100)}%…`;
    } else {
      progress.removeAttribute('value');
      status.textContent = `正在下载更新 ${(event.downloaded / 1048576).toFixed(1)} MB…`;
    }
  }).then(() => checkNow(true)).catch(() => {
    status.textContent = '更新组件暂时不可用，请重新启动应用后重试。';
  });
}
