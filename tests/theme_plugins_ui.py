"""Theme runtime regression in real Vite/browser; no backend or personal data.
Run with Python Playwright against http://127.0.0.1:1420 (or TWINLINK_TEST_URL).
"""
import json
import os
from playwright.sync_api import sync_playwright, expect

URL = os.environ.get('TWINLINK_TEST_URL', 'http://127.0.0.1:1420')

with sync_playwright() as p:
    browser = p.chromium.launch(headless=True)
    page = browser.new_page(viewport={'width': 1160, 'height': 780}, color_scheme='light')
    errors = []
    page.on('pageerror', lambda error: errors.append(str(error)))
    page.goto(URL)
    page.wait_for_load_state('networkidle')
    page.evaluate("async () => { window.T = await import('/src/themes.ts'); T.initTheme(); T.resetAppearance(); }")
    assert page.evaluate('T.listThemes().length') >= 3
    assert page.evaluate('T.getAppearance().mode') == 'system'
    assert page.evaluate('T.resolveMode()') == 'light'
    page.emulate_media(color_scheme='dark')
    page.wait_for_function("document.documentElement.dataset.theme === 'dark'")
    page.evaluate("T.setMode('light')")
    page.emulate_media(color_scheme='light')
    page.emulate_media(color_scheme='dark')
    assert page.evaluate('T.resolveMode()') == 'light'
    page.evaluate("T.setMode('dark'); T.setAccent('#ffff00'); T.setFontSize(18)")
    assert page.evaluate('T.getAppearance().fontSize') == 18
    assert page.evaluate('T.setAccent("url(https://bad.invalid/x)")') is False
    assert page.evaluate('T.setFontSize(100)') is False
    theme = page.evaluate("JSON.parse(T.exportThemeJson())")
    assert theme['style']['fontSize'] == 18, 'export should include effective font override'
    theme['id'] = 'qa-portable-theme'
    theme['name'] = '验收主题'
    theme['light']['canvas'] = '#f8fafc'
    theme['dark']['canvas'] = '#181b23'
    installed = page.evaluate('(theme) => T.installTheme(JSON.stringify(theme))', theme)
    assert installed['ok'], installed
    assert page.evaluate("T.setTheme('qa-portable-theme')")
    page.reload()
    page.wait_for_load_state('networkidle')
    page.evaluate("async () => { window.T = await import('/src/themes.ts'); T.initTheme(); }")
    assert page.evaluate('T.getAppearance().themeId') == 'qa-portable-theme'
    assert page.evaluate('T.getAppearance().fontSize') == 18
    assert page.evaluate('getComputedStyle(document.documentElement).getPropertyValue("--tl-canvas").trim()') == '#181b23'
    snapshot = page.evaluate('JSON.stringify({appearance:T.getAppearance(), themes:T.listThemes()})')
    invalid = [
        '{broken json',
        json.dumps({**theme, 'schemaVersion': 999}),
        json.dumps({**theme, 'script': 'alert(1)'}),
        json.dumps({**theme, 'light': {**theme['light'], 'backgroundImage': 'url(https://bad.invalid/x)'}}),
        json.dumps({**theme, 'dark': {**theme['dark'], 'accent': '#fff; color:red'}}),
        json.dumps({**theme, 'style': {'fontSize': 9000, 'radius': 12}}),
        json.dumps({**theme, '__proto__': {'polluted': True}}),
        json.dumps({**theme, 'id': 'dsh'}),
        ' ' * 65537,
    ]
    for raw in invalid:
        result = page.evaluate('(raw) => T.installTheme(raw)', raw)
        assert not result['ok'], raw[:80]
        assert page.evaluate('JSON.stringify({appearance:T.getAppearance(), themes:T.listThemes()})') == snapshot
    assert page.evaluate('({}).polluted') is None
    duplicate = page.evaluate('(theme) => T.installTheme(theme)', theme)
    assert not duplicate['ok'] and duplicate.get('conflict')
    theme['dark']['canvas'] = '#202630'
    result = page.evaluate('(theme) => T.installTheme(theme, {replace:true})', theme)
    assert result['ok'] and result['replaced']
    assert page.evaluate('getComputedStyle(document.documentElement).getPropertyValue("--tl-canvas").trim()') == '#202630', 'replacing active theme should repaint immediately'
    result = page.evaluate("T.uninstallTheme('qa-portable-theme')")
    assert result['ok'] and result['fellBack']
    assert page.evaluate('T.getAppearance().themeId') == 'dsh'
    assert not page.evaluate("T.uninstallTheme('dsh')")['ok']
    page.evaluate("localStorage.setItem(T.STORAGE_KEY, '{invalid')")
    page.reload()
    page.wait_for_load_state('networkidle')
    page.evaluate("async () => { window.T = await import('/src/themes.ts'); T.initTheme(); }")
    assert page.evaluate('T.getAppearance().themeId') == 'dsh'
    assert page.evaluate('T.getAppearance().mode') == 'system'
    # Exercise the real appearance UI, including explicit confirmation. The
    # backend stays in browser preview; only small user-selected JSON is read.
    page.locator('.appearance-open').first.click()
    def import_file(value, name='qa-theme.json'):
        page.locator('#theme-file').set_input_files({'name': name, 'mimeType': 'application/json', 'buffer': json.dumps(value).encode()})
    import_file(theme)
    page.wait_for_function("T.getAppearance().themeId === 'qa-portable-theme'")
    assert page.locator('.theme-card .theme-remove').count() == 0, 'deletion must be a sibling button'
    original = theme['dark']['canvas']
    theme['dark']['canvas'] = '#252b35'
    import_file(theme, 'replacement.json')
    expect(page.locator('#theme-confirm')).to_be_visible()
    assert page.evaluate('T.getActiveTheme().dark.canvas') == original
    page.locator('#theme-confirm-cancel').click()
    assert page.evaluate('T.getActiveTheme().dark.canvas') == original
    import_file(theme, 'replacement.json')
    page.locator('#theme-confirm-accept').click()
    assert page.evaluate('T.getActiveTheme().dark.canvas') == '#252b35'
    with page.expect_download() as download:
        page.locator('#theme-export').click()
    portable = json.loads(open(download.value.path()).read())
    assert page.evaluate('(pack) => T.parseTheme(pack).ok', portable)
    remove = page.locator('.theme-remove[data-remove-id="qa-portable-theme"]')
    remove.click()
    expect(page.locator('#theme-confirm')).to_be_visible()
    page.locator('#theme-confirm-cancel').click()
    assert page.evaluate('T.getAppearance().themeId') == 'qa-portable-theme'
    remove.click()
    page.locator('#theme-confirm-accept').click()
    assert page.evaluate('T.getAppearance().themeId') == 'dsh'
    expect(page.locator('.theme-card[data-theme-id="qa-portable-theme"]')).to_have_count(0)
    page.locator('#theme-json').fill(json.dumps(theme))
    page.locator('#theme-apply').click()
    page.wait_for_function("T.getAppearance().themeId === 'qa-portable-theme'")
    page.locator('#theme-json').fill('{broken json')
    page.locator('#theme-apply').click()
    expect(page.locator('#theme-error')).not_to_be_empty()
    assert page.evaluate('T.getAppearance().themeId') == 'qa-portable-theme'
    page.locator('#theme-reset').click()
    assert page.evaluate('T.getAppearance().themeId') == 'dsh'
    assert page.evaluate('T.getAppearance().mode') == 'system'
    page.keyboard.press('Escape')
    expect(page.locator('.preview-strip')).to_be_visible()
    assert not errors, errors
    browser.close()
print('PASS theme modes, system changes, custom preferences, portable export/install, refresh persistence, validation atomicity, replacement, uninstall and corrupt storage')
