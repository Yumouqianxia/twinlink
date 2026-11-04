"""Attachment UI regressions with explicit IPC/media fixtures, not real transfers.
Core tests exercise real loopback bytes and local preview authorization.
Run against Vite: python tests/attachments_ui.py [http://127.0.0.1:1420]
"""
import base64
import sys
from playwright.sync_api import sync_playwright, expect

URL = sys.argv[1] if len(sys.argv) > 1 else 'http://127.0.0.1:1420'
PNG = base64.b64decode('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jRZkAAAAASUVORK5CYII=')
FIXTURE = r'''
const room = {role:'host',roomId:'a'.repeat(64),networkName:'fixture',virtualIp:'10.1.1.1',peerVirtualIp:'10.1.1.2'};
const stamp = 1700000000000;
window.fixture = {
  profile:{displayName:'测试设备',downloadDir:'/fixture',autoAcceptFiles:false},
  room,connection:'connected',peerName:'测试伙伴',preview:false,engineAvailable:true,
  paused:false,reconnect:null,server:null,route:'direct',mailbox:{state:'online',pending:0},
  messages:[{id:'before',text:'文件之前',outgoing:false,timestamp:stamp,delivery:'delivered'},{id:'after',text:'文件之后',outgoing:true,timestamp:stamp+2000,delivery:'delivered'}],
  transfers:[{id:'11111111-1111-4111-8111-111111111111',name:'截图.png',size:68,transferred:68,outgoing:true,status:'waiting',via:'direct',timestamp:stamp+1000},
    {id:'22222222-2222-4222-8222-222222222222',name:'对方图片.png',size:68,transferred:0,outgoing:false,status:'offered',via:'direct',timestamp:stamp+3000}]
};
window.calls=[];
const log=(method,args)=>window.calls.push({method,args});
export const api={
 isDesktop:()=>true,snapshot:async()=>structuredClone(window.fixture),
 subscribe:async handler=>{window.appEvent=handler;return()=>{}},
 onResync:async handler=>{window.resync=handler;return()=>{}},
 onFileDrop:async handler=>{window.nativeDrop=handler;return()=>{}},
 onUpdateProgress:async()=>()=>{},checkUpdate:async()=>({currentVersion:'0.0.2',version:null,notes:null}),
 imagePreviewUrl:(room,id)=>'/__fixture_image__/'+room+'/'+id,
 chooseImage:async()=>{log('chooseImage');return '/fixture/新图片.png'},
 chooseFile:async()=>{log('chooseFile');return '/fixture/文档.txt'},
 offerFile:async path=>{log('offerFile',path);const t={id:crypto.randomUUID(),name:path.split('/').pop(),size:68,transferred:0,outgoing:true,status:'waiting',via:'direct',timestamp:stamp+4000};window.fixture.transfers.push(t);return structuredClone(t)},
 acceptFile:async id=>log('accept',id),rejectFile:async id=>log('reject',id),cancelTransfer:async id=>log('cancel',id)
};
'''

with sync_playwright() as p:
    browser = p.chromium.launch(headless=True)
    page = browser.new_page(viewport={'width':1160,'height':780})
    errors = []
    page.on('pageerror', lambda err: errors.append(str(err)))
    page.route('**/src/api.ts*', lambda route: route.fulfill(status=200, content_type='application/javascript', body=FIXTURE))
    media_reads = []
    def media(route):
        media_reads.append(route.request.url)
        route.fulfill(status=200, content_type='image/png', headers={'Cache-Control':'no-store'}, body=PNG)
    page.route('**/__fixture_image__/**', media)
    page.goto(URL)
    page.wait_for_load_state('networkidle')
    expect(page.locator('.messages > li')).to_have_count(4)
    assert page.locator('.messages > li').evaluate_all('(nodes)=>nodes.map(n=>n.dataset.id)') == ['before','11111111-1111-4111-8111-111111111111','after','22222222-2222-4222-8222-222222222222']
    incoming = page.locator('.attachment-msg.in')
    expect(incoming.get_by_role('button', name='接收', exact=True)).to_be_visible()
    assert incoming.locator('img').count() == 0
    assert not any('22222222' in url for url in media_reads), 'unaccepted images must not be fetched'
    incoming.get_by_role('button', name='接收', exact=True).click()
    assert page.evaluate("calls.some(c=>c.method==='accept')")
    page.locator('.image-thumbnail').first.click()
    expect(page.locator('#dlg-image')).to_be_visible()
    page.wait_for_function("document.querySelector('#image-full').naturalWidth > 0")
    page.keyboard.press('Escape')
    expect(page.locator('#dlg-image')).to_be_hidden()
    page.wait_for_function("!document.querySelector('#image-full').hasAttribute('src')")
    page.evaluate("window.savedThumbnail=document.querySelector('.attachment-msg.out img')")
    reads_before = len(media_reads)
    page.evaluate("appEvent({type:'transfer',transfer:{...fixture.transfers[0],status:'completed'}})")
    assert page.evaluate("savedThumbnail===document.querySelector('.attachment-msg.out img')")
    assert len(media_reads) == reads_before
    page.evaluate("appEvent({type:'transfer',transfer:{...fixture.transfers[1],status:'completed',transferred:68}})")
    expect(incoming.locator('img')).to_be_visible()
    expect(incoming).not_to_contain_text('接收完成后可预览')
    page.locator('.attach-button').click()
    page.locator('.image-button').click()
    expect(page.locator('.page-chat')).to_be_visible()
    expect(page.locator('.messages')).to_contain_text('文档.txt')
    expect(page.locator('.messages')).to_contain_text('新图片.png')
    assert page.evaluate("calls.filter(c=>c.method==='offerFile').length") == 2
    page.reload()
    page.wait_for_load_state('networkidle')
    expect(page.locator('.messages')).to_contain_text('截图.png')
    # Progress events preserve an older-history reader's scroll anchor.
    page.evaluate("for(let i=0;i<40;i++)appEvent({type:'message',message:{id:'scroll-'+i,text:'历史消息 '+i,timestamp:1700000005000+i,outgoing:false,delivery:'delivered'}})")
    page.locator('.thread').evaluate('(el)=>el.scrollTop=100')
    top = page.locator('.thread').evaluate('(el)=>el.scrollTop')
    page.evaluate("appEvent({type:'transfer',transfer:{...fixture.transfers[0],status:'sending',transferred:30}})")
    assert abs(page.locator('.thread').evaluate('(el)=>el.scrollTop')-top) < 2
    # Return to compact fixture and capture both palettes.
    page.reload()
    page.wait_for_load_state('networkidle')
    page.locator('.thread').evaluate('(el)=>el.scrollTop=el.scrollHeight')
    page.emulate_media(color_scheme='dark')
    page.screenshot(path='/tmp/twinlink-dsh-attachments/chat-dark.png', full_page=True, animations='disabled')
    page.emulate_media(color_scheme='light')
    page.screenshot(path='/tmp/twinlink-dsh-attachments/chat-light.png', full_page=True, animations='disabled')
    # A removed source image degrades without an uncaught page exception.
    page.evaluate("document.querySelector('.attachment-msg.out img').dispatchEvent(new Event('error'))")
    expect(page.locator('.attachment-msg.out .image-fallback').first).to_be_visible()
    # Room switch invalidates both the transcript and an open image dialog.
    page.evaluate("appEvent({type:'transfer',transfer:{...fixture.transfers[1],status:'completed',transferred:68}})")
    page.locator('.image-thumbnail:not([hidden])').last.click()
    page.evaluate("resync({...fixture,room:{...fixture.room,roomId:'b'.repeat(64)},messages:[],transfers:[]})")
    expect(page.locator('#dlg-image')).to_be_hidden()
    expect(page.locator('.messages')).to_be_hidden()
    assert not errors, errors
    page.close()
    # Actual browser preview: no IPC fixtures or simulated attachment records.
    preview = browser.new_page(viewport={'width':880,'height':620})
    preview.goto(URL)
    preview.wait_for_load_state('networkidle')
    expect(preview.locator('.preview-strip')).to_be_visible()
    for color in ['light','dark']:
        preview.emulate_media(color_scheme=color)
        buttons = preview.locator('.empty-actions .btn').all()
        rects = [button.bounding_box() for button in buttons]
        assert len(rects) == 2
        assert abs(rects[0]['y']-rects[1]['y']) < 1
        assert abs(rects[0]['height']-rects[1]['height']) < 1
        assert not preview.evaluate('document.documentElement.scrollWidth>innerWidth')
        preview.screenshot(path=f'/tmp/twinlink-dsh-attachments/aligned-{color}.png', full_page=True, animations='disabled')
    expect(preview.locator('.attach-button')).to_be_disabled()
    expect(preview.locator('.image-button')).to_be_disabled()
    browser.close()
print('PASS ordered attachment timeline, consent/actions, image dialog/Escape, stable thumbnails and scroll, room reset, refresh fixture, separate pickers, aligned empty actions, real preview guard (IPC/media fixtures, not native GUI)')
