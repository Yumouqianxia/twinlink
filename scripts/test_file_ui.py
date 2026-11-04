"""UI wiring regression only; IPC/dialog/drop fixtures are explicitly mocked.
Run against Vite with Python Playwright installed:
  python scripts/test_file_ui.py http://127.0.0.1:1420
Real TCP and mailbox file bytes are covered by Rust tests, not these fixtures.
"""
import sys
from playwright.sync_api import sync_playwright, expect

BASE = sys.argv[1] if len(sys.argv) > 1 else "http://127.0.0.1:1420"
FIXTURE = r'''
const snapshot = {
  profile: { displayName: '测试设备', downloadDir: '/fixture/received', autoAcceptFiles: false },
  connection: 'connected', peerName: '测试伙伴', messages: [], transfers: [],
  engineAvailable: true, preview: false, paused: false, reconnect: null,
  server: {relayUrl:'tcp://fixture:1',mailboxUrl:'https://fixture:2',mailboxHost:'fixture'},
  room: {role:'host',roomId:'a'.repeat(64),networkName:'fixture',virtualIp:'10.1.1.1',peerVirtualIp:'10.1.1.2'},
  route: 'direct', mailbox: {state:'online',message:null,pending:0}
};
window.__calls = [];
const log = (method, args) => window.__calls.push({method, args});
export const api = {
  isDesktop: () => true,
  snapshot: async () => structuredClone(snapshot),
  subscribe: async h => { window.__appEvent = h; return () => {}; },
  onResync: async () => () => {},
  onUpdateProgress: async () => () => {},
  onFileDrop: async h => {window.__nativeDrop = h; return () => {};},
  chooseFile: async () => '/fixture/picked.txt',
  chooseFolder: async () => '/fixture/received',
  offerFile: async path => {
    log('offerFile', path);
    return {id:crypto.randomUUID(),name:path.split('/').pop(),size:42,transferred:0,outgoing:true,status:'waiting',via:'direct'};
  },
  saveProfile: async profile => { log('saveProfile',profile); snapshot.profile=profile; },
  chooseExport: async () => '/fixture/export.json',
  exportData: async path => {log('exportData',path);},
  chooseBackup: async () => '/fixture/import.json',
  pause: async () => {log('pause'); snapshot.paused=true;snapshot.connection='disconnected';},
  importData: async path => {log('importData',path);snapshot.profile.autoAcceptFiles=false;return {messagesAdded:205,transfersAdded:2,restoredConnection:false};}
};
'''

with sync_playwright() as p:
    browser = p.chromium.launch(headless=True)
    page = browser.new_page(viewport={"width": 1160, "height": 780})
    errors = []
    page.on("pageerror", lambda error: errors.append(str(error)))
    page.route("**/src/api.ts*", lambda route: route.fulfill(status=200, content_type="application/javascript", body=FIXTURE))
    page.goto(BASE)
    page.wait_for_load_state("networkidle")
    page.wait_for_function("typeof window.__nativeDrop === 'function'")
    page.evaluate("window.__nativeDrop({type:'enter',paths:['/fixture/拖拽.txt']})")
    expect(page.locator("#file-drop-hint")).to_be_visible()
    page.evaluate("window.__nativeDrop({type:'leave'})")
    expect(page.locator("#file-drop-hint")).to_be_hidden()
    page.evaluate("window.__nativeDrop({type:'drop',paths:['/fixture/拖拽.txt','/fixture/second.txt','/fixture/拖拽.txt']})")
    page.wait_for_function("window.__calls.filter(c=>c.method==='offerFile').length === 2")
    expect(page.locator(".transfers")).to_contain_text("拖拽.txt")
    expect(page.locator(".transfers")).to_contain_text("P2P 直连")
    page.locator(".settings-open").first.click()
    expect(page.locator("#dlg-settings")).to_be_visible()
    page.evaluate("window.__nativeDrop({type:'drop',paths:['/fixture/private-backup.json']})")
    assert page.evaluate("window.__calls.filter(c=>c.method==='offerFile').length") == 2
    expect(page.locator("[name=autoAcceptFiles]")).not_to_be_checked()
    page.locator("[name=autoAcceptFiles]").check()
    page.locator("#settings-form button[type=submit]").click()
    page.wait_for_function("window.__calls.some(c=>c.method==='saveProfile')")
    assert page.evaluate("window.__calls.find(c=>c.method==='saveProfile').args.autoAcceptFiles") is True
    page.locator(".settings-open").first.click()
    page.locator("#export-data").click()
    expect(page.locator("#backup-status")).to_contain_text("备份已保存")
    page.locator("#import-data").click()
    expect(page.locator("#import-data")).to_have_text("确认导入并暂停连接")
    assert not page.evaluate("window.__calls.some(c=>c.method==='importData')")
    page.locator("#cancel-import").click()
    expect(page.locator("#import-data")).to_have_text("选择备份导入")
    page.locator("#import-data").click()
    page.locator("#import-data").click()
    expect(page.locator("#backup-status")).to_contain_text("新增 205 条聊天、2 条文件记录")
    expect(page.locator("[name=autoAcceptFiles]")).not_to_be_checked()
    calls = page.evaluate("window.__calls.map(c=>c.method)")
    assert calls.index("pause") < calls.index("importData")
    assert not errors, errors
    page.screenshot(path="/tmp/twinlink-file-settings-qa.png", full_page=True)
    page.close()
    # Production browser preview: no mocked desktop APIs or successful transfers.
    preview = browser.new_page()
    preview.on("pageerror", lambda error: errors.append(str(error)))
    preview.goto(BASE)
    preview.wait_for_load_state("networkidle")
    expect(preview.locator(".preview-strip")).to_be_visible()
    preview.evaluate("document.dispatchEvent(new DragEvent('drop', {bubbles:true,cancelable:true}))")
    expect(preview.locator(".toast")).to_contain_text("拖拽发送文件需要使用桌面应用")
    assert not errors, errors
    browser.close()
print("PASS native-drop wiring, deduplication, dialog exclusion, explicit auto-accept, backup confirmation, merge result and real preview guard (IPC fixtures; no native GUI claim)")
