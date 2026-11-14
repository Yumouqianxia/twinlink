"""Message-delivery UI regressions with an explicit IPC fixture.

The fixture replays the exact ordering that made a message stay "尚未确认送达" long
after the peer had already confirmed it: the peer's receipt (a `message` event)
arrives *before* the `sendMessage()` reply, which still carries the raw pending
copy. Delivery state must only ever move forward, so the late reply may not
demote "已送达对方设备" back to "尚未确认送达".

No real transfers, sockets or user data: only fake IPC payloads.
Run against Vite: python tests/message_delivery_ui.py [http://127.0.0.1:1420]
"""
import os
import sys
from playwright.sync_api import sync_playwright, expect

URL = sys.argv[1] if len(sys.argv) > 1 else 'http://127.0.0.1:1420'
SHOTS = '/tmp/twinlink-dsh-delivery'
FIXTURE = r'''
const stamp = 1700000000000;
const roomA = {role:'host',roomId:'a'.repeat(64),networkName:'fixture',virtualIp:'10.1.1.1',peerVirtualIp:'10.1.1.2'};
const roomB = {role:'join',roomId:'b'.repeat(64),networkName:'fixture-b',virtualIp:'10.2.2.2',peerVirtualIp:'10.2.2.1'};
window.roomA = roomA;
window.roomB = roomB;
window.fixture = {
  profile:{displayName:'测试设备',downloadDir:'/fixture',autoAcceptFiles:false},
  room:roomA,connection:'connected',peerName:'测试伙伴',preview:false,engineAvailable:true,
  paused:false,reconnect:null,server:null,route:'direct',
  mailbox:{state:'online',message:null,pending:0},
  messages:[{id:'old',text:'较早的消息',outgoing:true,timestamp:stamp,delivery:'delivered',via:'direct'}],
  transfers:[]
};
window.calls=[];
const log=(method,args)=>window.calls.push({method,args});
const copy=()=>structuredClone(window.fixture);
export const api={
  isDesktop:()=>true,
  // The copy is taken when the call is made, like a real command reading the
  // database before a slow reply; the test can hold the reply back.
  snapshot:async()=>{
    log('snapshot');
    const snap = copy();
    if (window.holdSnapshots) await new Promise(resolve => window.pendingSnapshots.push(resolve));
    return snap;
  },
  subscribe:async handler=>{window.appEvent=handler;return()=>{}},
  onResync:async handler=>{window.resync=handler;return()=>{}},
  onFileDrop:async handler=>{window.nativeDrop=handler;return()=>{}},
  onUpdateProgress:async()=>()=>{},
  checkUpdate:async()=>({currentVersion:'0.0.3',version:null,notes:null}),
  history:async before=>{log('history',before);return {roomId:window.fixture.room.roomId,messages:[],hasMore:false}},
  imagePreviewUrl:(room,id)=>'/__fixture_image__/'+room+'/'+id,
  chooseImage:async()=>null,chooseFile:async()=>null,chooseFolder:async()=>null,
  offerFile:async path=>{log('offerFile',path);return null},
  acceptFile:async id=>log('accept',id),rejectFile:async id=>log('reject',id),cancelTransfer:async id=>log('cancel',id),
  // Mirrors the core: the pending message is emitted *before* the command
  // returns, and the command's own reply still carries the pending copy.
  sendMessage:async text=>{
    log('sendMessage',text);
    const pending={id:'new',text,outgoing:true,timestamp:stamp+1000,delivery:'pending'};
    window.fixture.messages.push(pending);
    window.appEvent({type:'message', message:structuredClone(pending)});
    if (window.sendGate) await window.sendGate;
    return structuredClone(pending);
  },
  pause:async()=>log('pause'),resume:async()=>log('resume'),forgetRoom:async()=>log('forget'),
  createRoom:async()=>'TL1.fixture-code',joinRoom:async code=>log('join',code),
  saveProfile:async profile=>{window.fixture.profile=profile;return true},
  exportData:async()=>{},importData:async()=>({messagesAdded:0,transfersAdded:0})
};
'''

def row(page, message_id):
    return page.locator(f'.messages > li[data-id="{message_id}"]')

def status(page, message_id):
    return row(page, message_id).locator('.msg-status')

def open_page(page):
    page.goto(URL)
    page.wait_for_load_state('networkidle')

with sync_playwright() as p:
    os.makedirs(SHOTS, exist_ok=True)
    browser = p.chromium.launch(headless=True)
    page = browser.new_page(viewport={'width': 1160, 'height': 780})
    errors = []
    page.on('pageerror', lambda err: errors.append(str(err)))
    page.route('**/src/api.ts*', lambda route: route.fulfill(status=200, content_type='application/javascript', body=FIXTURE))
    open_page(page)

    # --- 1. the reported regression: receipt first, stale pending reply second.
    page.evaluate("""() => {
      let release;
      window.sendGate = new Promise(resolve => { release = resolve; });
      window.releaseSend = () => { window.sendGate = null; release(); };
    }""")
    page.fill('#message-input', '回执先到')
    page.press('#message-input', 'Enter')
    expect(status(page, 'new')).to_have_text('尚未确认送达')
    screenshot_pending = os.path.join(SHOTS, '01-pending-before-receipt.png')
    page.screenshot(path=screenshot_pending)

    # The peer confirms the message while the send call is still outstanding.
    page.evaluate("""() => {
      window.fixture.messages = window.fixture.messages.map(m =>
        m.id === 'new' ? {...m, delivery:'delivered', via:'mailbox'} : m);
      window.appEvent({type:'message', message:{id:'new', text:'回执先到', outgoing:true,
        timestamp: window.fixture.messages.find(m=>m.id==='new').timestamp,
        delivery:'delivered', via:'mailbox'}});
    }""")
    expect(status(page, 'new')).to_have_text('已送达对方设备')

    # The late reply carries the raw pending copy: it must not demote the row.
    page.evaluate("() => window.releaseSend()")
    page.wait_for_timeout(150)
    assert status(page, 'new').inner_text() == '已送达对方设备', 'a late send reply must not regress delivery'
    assert page.locator('.messages > li[data-id="new"]').count() == 1, 'no duplicate row'
    assert '对方设备已确认收到' in status(page, 'new').get_attribute('title')
    assert '已读' in status(page, 'new').get_attribute('title')
    page.screenshot(path=os.path.join(SHOTS, '02-delivered-after-late-reply.png'))

    # --- 2. a stale snapshot must not roll a delivered row back either.
    page.evaluate("""() => {
      window.fixture.messages = window.fixture.messages.map(m =>
        m.id === 'new' ? {...m, delivery:'pending', via:undefined} : m);
      window.resync(structuredClone(window.fixture));
    }""")
    page.wait_for_timeout(120)
    assert status(page, 'new').inner_text() == '已送达对方设备', 'a stale snapshot must not regress delivery'
    assert status(page, 'new').get_attribute('title').endswith('离线信箱') or '离线信箱' in status(page, 'new').get_attribute('title'), 'the confirmed transport must survive'

    # --- 3. an older message keeps chronological order, a stored copy is
    # promoted by a real receipt, and the original timestamp/identity survives.
    page.evaluate("""() => {
      window.appEvent({type:'message', message:{id:'older', text:'更早一条', outgoing:true,
        timestamp: 1699000000000, delivery:'stored', via:'mailbox'}});
    }""")
    page.wait_for_timeout(80)
    assert status(page, 'older').inner_text() == '服务器已暂存'
    assert '不代表已读' in status(page, 'older').get_attribute('title')
    page.evaluate("""() => {
      window.appEvent({type:'message', message:{id:'older', text:'更早一条', outgoing:true,
        timestamp: 1699000000000, delivery:'delivered', via:'direct'}});
    }""")
    page.wait_for_timeout(80)
    ids = page.locator('.messages > li').evaluate_all("nodes => nodes.map(n => n.dataset.id)")
    assert ids == ['older', 'old', 'new'], f'unexpected order: {ids}'
    assert status(page, 'older').inner_text() == '已送达对方设备'
    assert row(page, 'older').locator('time').get_attribute('datetime') == page.evaluate("new Date(1699000000000).toISOString()"), 'the original timestamp must be kept'

    # --- 6. no fabricated read/online state in the delivery labels.
    labels = page.locator('.msg-status').evaluate_all("nodes => nodes.map(n => n.textContent)")
    assert labels and all(l in ('尚未确认送达', '服务器已暂存', '已送达对方设备', '未确认') for l in labels), labels
    assert page.locator('.msg-status').evaluate_all("nodes => nodes.every(n => n.title.includes('不代表已读') || n.title.includes('没有保存送达状态'))")

    # --- 4. room switch: an in-flight snapshot for the old room is discarded.
    # `refresh()` (the pause button) issues a snapshot request; the room switch
    # lands before it resolves, and the stale reply must not seed room B.
    page.evaluate("""() => {
      window.pendingSnapshots = [];
      window.holdSnapshots = true;
      window.releaseSnapshot = index => window.pendingSnapshots.splice(index, 1)[0]();
      window.fixture.connection = 'disconnected';
      window.fixture.paused = true;
      window.appEvent({type:'state', connection:'disconnected', peerName:'测试伙伴', paused:true,
        reconnect:null, room: window.roomA});
    }""")
    page.wait_for_timeout(60)
    page.click('.conn-action')          # resume() -> refresh(), snapshot of room A held
    page.wait_for_timeout(60)
    page.evaluate("""() => {
      window.fixture.room = window.roomB;
      window.fixture.messages = [];
      window.fixture.peerName = '另一个伙伴';
      window.fixture.connection = 'connected';
      window.fixture.paused = false;
      window.appEvent({type:'state', connection:'connected', peerName:'另一个伙伴', paused:false,
        reconnect:null, room: window.roomB});
    }""")
    page.wait_for_timeout(80)
    assert page.locator('.messages > li').count() == 0, 'the new room starts empty, not seeded from the old one'
    # Room B's own snapshot arrives first, then the stale room A reply.
    page.evaluate("() => window.releaseSnapshot(1)")
    page.wait_for_timeout(80)
    page.evaluate("() => window.releaseSnapshot(0)")
    page.wait_for_timeout(150)
    assert page.locator('.messages > li').count() == 0, 'a late snapshot for the previous room must be dropped'
    assert row(page, 'new').count() == 0, 'the previous transcript must not come back'
    assert '另一个伙伴' in page.locator('#page-sub').inner_text(), 'the room identity must not revert'
    assert '断开连接' in page.locator('.conn-action').inner_text(), 'the connection state must not revert'

    # --- 5. mailbox wording: queued items are not one chat message each.
    page.evaluate("""() => {
      window.appEvent({type:'mailbox', state:'online', message:null, pending:3});
    }""")
    page.wait_for_timeout(80)
    assert '3 项待同步' in page.locator('.mailbox-text').inner_text()
    assert '条待发' not in page.locator('.mailbox').inner_text()
    page.screenshot(path=os.path.join(SHOTS, '03-mailbox-sync-count.png'))


    # --- 6. a server-stored event also survives a late pending invoke reply.
    page.reload()
    page.wait_for_load_state('networkidle')
    page.evaluate("() => {window.sendGate=new Promise(resolve=>window.releaseStoredSend=resolve);}")
    page.fill('#message-input', '服务器先确认')
    page.press('#message-input', 'Enter')
    expect(status(page, 'new')).to_have_text('尚未确认送达')
    page.evaluate("appEvent({type:'message',message:{id:'new',text:'过期内容不应覆盖正文',outgoing:false,timestamp:1,delivery:'stored',via:'mailbox'}})")
    page.evaluate("window.releaseStoredSend()")
    expect(status(page, 'new')).to_have_text('服务器已暂存')
    expect(row(page, 'new').locator('.msg-bubble')).to_have_text('服务器先确认')
    assert 'out' in row(page, 'new').get_attribute('class')
    page.evaluate('window.resync(structuredClone(window.fixture))')
    expect(status(page, 'new')).to_have_text('服务器已暂存')
    assert row(page, 'new').locator('time').get_attribute('datetime') == page.evaluate('new Date(1700000001000).toISOString()')

    # --- 7. a response from a previous room cannot seed the current room or
    # erase the new room's draft, even if the old send operation was pending.
    page.reload()
    page.wait_for_load_state('networkidle')
    page.evaluate('() => {window.sendGate=new Promise(resolve=>window.releaseOldSend=resolve);}')
    page.fill('#message-input', '旧房间的发送')
    page.press('#message-input', 'Enter')
    expect(status(page, 'new')).to_have_text('尚未确认送达')
    page.evaluate("""() => {
      fixture.room=roomB;fixture.messages=[];
      appEvent({type:'state',connection:'connected',peerName:'新伙伴',paused:false,reconnect:null,room:roomB});
    }""")
    expect(page.locator('.messages > li')).to_have_count(0)
    page.fill('#message-input', '新房间草稿')
    page.evaluate('window.releaseOldSend()')
    page.wait_for_function("document.querySelector('.composer-hint').textContent.includes('Enter')")
    expect(page.locator('.messages > li')).to_have_count(0)
    expect(page.locator('#message-input')).to_have_value('新房间草稿')

    assert not errors, f'page errors: {errors}'
    print('message delivery UI regressions passed')
    browser.close()
