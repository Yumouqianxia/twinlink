"""UI regression only: real Vite UI, mocked Tauri IPC; no installer is run.
Run against a local dev server with Python 3 + playwright and Chromium installed.
"""
import json
import os
from pathlib import Path
from playwright.sync_api import sync_playwright, expect

URL = os.environ.get('TWINLINK_TEST_URL', 'http://127.0.0.1:1420')
PACKAGE_VERSION = json.loads((Path(__file__).resolve().parents[1] / 'package.json').read_text())['version']
SNAPSHOT = {'profile': {'displayName': 'Test device', 'downloadDir': ''}, 'connection': 'disconnected', 'peerName': None, 'messages': [], 'transfers': [], 'engineAvailable': True, 'preview': False, 'paused': False, 'reconnect': None, 'server': None, 'room': None, 'route': 'unknown', 'mailbox': {'state': 'notConfigured', 'message': None, 'pending': 0}}
INIT = r"""
window.isTauri = true;
window.testCalls = [];
window.testCallbacks = new Map();
window.testEvents = {};
window.__TAURI_INTERNALS__ = {
 metadata: {currentWindow: {label: 'main'}, currentWebview: {label: 'main'}},
 transformCallback(cb) { const id = window.testCallbacks.size + 1; window.testCallbacks.set(id, cb); return id; },
 unregisterCallback() {},
 invoke(cmd, args) {
  window.testCalls.push(cmd);
  if(cmd === 'plugin:event|listen') { window.testEvents[args.event] = window.testCallbacks.get(args.handler); return Promise.resolve(1); }
  if(cmd === 'plugin:event|unlisten') return Promise.resolve();
  if(cmd === 'snapshot') return Promise.resolve(window.testSnapshot);
  if(cmd === 'check_update') return window.testMode === 'error' ? Promise.reject('GitHub unavailable') : Promise.resolve({currentVersion:'0.0.1', version:window.testMode === 'latest' ? null : '0.0.2', notes:'<img src=x onerror=alert(1)> Release notes'});
  if(cmd === 'install_update') return Promise.reject('Signature verification failed; not installed');
  return Promise.resolve(null);
 }
};
"""

with sync_playwright() as p:
    browser = p.chromium.launch(headless=True)
    page = browser.new_page(viewport={'width': 1160, 'height': 780})
    errors = []
    page.on('pageerror', lambda e: errors.append(str(e)))
    page.goto(URL)
    page.wait_for_load_state('networkidle')
    page.locator('.nav-item.settings-open').click()
    expect(page.locator('#check-update')).to_be_disabled()
    expect(page.locator('#update-status')).to_contain_text('界面预览')
    page.close()
    for mode in ['latest', 'available', 'error']:
        page = browser.new_page(viewport={'width': 1160, 'height': 780})
        page.on('pageerror', lambda e: errors.append(str(e)))
        page.add_init_script('window.testSnapshot = ' + json.dumps(SNAPSHOT) + '; window.testMode = ' + json.dumps(mode) + ';' + INIT)
        page.goto(URL)
        page.wait_for_load_state('networkidle')
        page.locator('.nav-item.settings-open').click()
        expect(page.locator('.update-version')).to_have_text(f"v{PACKAGE_VERSION if mode == 'error' else '0.0.1'} · 预览版")
        if mode == 'latest':
            expect(page.locator('#update-status')).to_contain_text('最新预览版')
            expect(page.locator('#install-update')).to_be_hidden()
        elif mode == 'error':
            expect(page.locator('#update-status')).to_contain_text('GitHub unavailable')
            expect(page.locator('#check-update')).to_be_enabled()
            expect(page.locator('#install-update')).to_be_hidden()
        else:
            expect(page.locator('#install-update')).to_have_text('更新至 v0.0.2 并重启')
            assert page.locator('#update-notes img').count() == 0
            assert page.evaluate("window.testCalls.includes('install_update')") is False
            page.locator('#install-update').click()
            expect(page.locator('#update-status')).to_contain_text('Signature verification failed')
            expect(page.locator('#install-update')).to_be_enabled()
            expect(page.locator('#check-update')).to_be_enabled()
        page.close()
    assert not errors, errors
    browser.close()
print('PASS preview, latest, available, safe notes, explicit install, failure/retry; no pageerror')
