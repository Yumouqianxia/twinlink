/**
 * TwinLink appearance runtime.
 *
 * Themes are declarative JSON documents ("theme plugins"): a manifest plus one
 * colour group per colour scheme. Nothing here evaluates code from a theme:
 * files are parsed, every field is validated, and only the whitelisted tokens
 * are copied into a fresh palette. Themes can therefore be authored, shared and
 * installed without touching `main.ts` or rebuilding the app.
 *
 * The module owns storage, validation, the system colour-scheme listener and the
 * mapping from semantic tokens to CSS custom properties. `main.ts` only renders
 * the appearance settings and calls this API.
 */

/* ---------- public types ---------- */

export type ThemeMode = 'light' | 'dark' | 'system';
export type ResolvedMode = 'light' | 'dark';

export const THEME_FORMAT = 'twinlink-theme';
export const THEME_SCHEMA_VERSION = 1 as const;
export const DEFAULT_THEME_ID = 'dsh';
export const MAX_THEME_BYTES = 64 * 1024;
export const MAX_USER_THEMES = 16;
export const STORAGE_KEY = 'twinlink.appearance.v1';
const STORAGE_VERSION = 1;

/** Semantic colour tokens every theme (and every colour scheme) must describe. */
export const TOKEN_KEYS = [
  'canvas',
  'sidebar',
  'surface',
  'surfaceAlt',
  'hover',
  'active',
  'ink',
  'muted',
  'faint',
  'line',
  'lineStrong',
  'accent',
  'accentHover',
  'accentPress',
  'accentSoft',
  'accentLine',
  'onAccent',
  'danger',
  'dangerSoft',
  'warn',
  'warnSoft',
  'warnLine',
  'ok',
  'okSoft',
] as const;

export type TokenName = (typeof TOKEN_KEYS)[number];
export type Palette = Record<TokenName, string>;

export interface ThemeStyle {
  /** Corner radius in px, 0–18. */
  radius: number;
  /** Suggested root font size in px, 12–18. */
  fontSize: number;
}

export interface ThemeManifest {
  format: typeof THEME_FORMAT;
  schemaVersion: typeof THEME_SCHEMA_VERSION;
  id: string;
  name: string;
  version: string;
  author?: string;
  description?: string;
  light: Palette;
  dark: Palette;
  style: ThemeStyle;
}

/** Read-only view handed to the settings UI. */
export interface ThemeSummary {
  id: string;
  name: string;
  version: string;
  author: string;
  description: string;
  builtin: boolean;
  style: ThemeStyle;
  light: Palette;
  dark: Palette;
}

export interface AppearanceState {
  mode: ThemeMode;
  resolved: ResolvedMode;
  themeId: string;
  /** `null` means "use the active theme's accent". */
  accent: string | null;
  /** `null` means "use the active theme's suggested size". */
  fontSize: number | null;
}

export type ThemeParseResult =
  | { ok: true; theme: ThemeManifest }
  | { ok: false; error: string };

export type ThemeInstallResult =
  | { ok: true; theme: ThemeManifest; replaced: boolean }
  | { ok: false; error: string; conflict?: ThemeManifest };

/* ---------- palettes ---------- */

const dshLight: Palette = {
  canvas: '#ffffff',
  sidebar: '#f9fafb',
  surface: '#ffffff',
  surfaceAlt: '#f5f6f7',
  hover: '#f1f3f5',
  active: '#ebeef2',
  ink: '#16181d',
  muted: '#61666b',
  faint: '#979da6',
  line: '#e6e9ee',
  lineStrong: '#cfd3d6',
  accent: '#3b6ee0',
  accentHover: '#2f5fc7',
  accentPress: '#274fae',
  accentSoft: '#eef3fe',
  accentLine: '#cfdcfa',
  onAccent: '#ffffff',
  danger: '#c0372c',
  dangerSoft: '#fdeeec',
  warn: '#8a5a00',
  warnSoft: '#fdf6e6',
  warnLine: '#ecd9a8',
  ok: '#1f7a4d',
  okSoft: '#eaf6ef',
};

const dshDark: Palette = {
  canvas: '#151517',
  sidebar: '#1b1b1c',
  surface: '#1f1f21',
  surfaceAlt: '#232324',
  hover: '#2c2c2e',
  active: '#353638',
  ink: '#f2f3f5',
  muted: '#a4a8ae',
  faint: '#7b7f86',
  line: '#2b2c2f',
  lineStrong: '#3d3e42',
  accent: '#7f9ef5',
  accentHover: '#93aeff',
  accentPress: '#6b8bea',
  accentSoft: '#1d2637',
  accentLine: '#2f4468',
  onAccent: '#101318',
  danger: '#f08a7d',
  dangerSoft: '#33201d',
  warn: '#e0b062',
  warnSoft: '#2c2416',
  warnLine: '#4a3a1c',
  ok: '#66c295',
  okSoft: '#16291f',
};

const forestLight: Palette = {
  canvas: '#ffffff',
  sidebar: '#f6f9f7',
  surface: '#ffffff',
  surfaceAlt: '#f2f6f3',
  hover: '#eef4f0',
  active: '#e4eee8',
  ink: '#15201a',
  muted: '#5b6a60',
  faint: '#93a29a',
  line: '#e3ebe6',
  lineStrong: '#cbd8d0',
  accent: '#2f7d5d',
  accentHover: '#276a4f',
  accentPress: '#1f5741',
  accentSoft: '#ecf6f0',
  accentLine: '#c9ddd2',
  onAccent: '#ffffff',
  danger: '#b23c2f',
  dangerSoft: '#fdeeeb',
  warn: '#7d5a11',
  warnSoft: '#fbf5e5',
  warnLine: '#e8d8ab',
  ok: '#1f7a4d',
  okSoft: '#eaf6ef',
};

const forestDark: Palette = {
  canvas: '#121614',
  sidebar: '#181d1a',
  surface: '#1c221e',
  surfaceAlt: '#202621',
  hover: '#27302a',
  active: '#2f3a33',
  ink: '#eef3f0',
  muted: '#a1aca5',
  faint: '#78847d',
  line: '#28312b',
  lineStrong: '#3a453e',
  accent: '#76c7a0',
  accentHover: '#8cd5b2',
  accentPress: '#63b48e',
  accentSoft: '#1b2b22',
  accentLine: '#2c4739',
  onAccent: '#0e1512',
  danger: '#ec8a7c',
  dangerSoft: '#31201c',
  warn: '#d8ac62',
  warnSoft: '#2a2417',
  warnLine: '#463a1d',
  ok: '#6dc59a',
  okSoft: '#16281f',
};

const graphiteLight: Palette = {
  canvas: '#fdfcfa',
  sidebar: '#f7f4ef',
  surface: '#ffffff',
  surfaceAlt: '#f6f2ec',
  hover: '#f0ebe4',
  active: '#e9e2d9',
  ink: '#1e1b17',
  muted: '#655f57',
  faint: '#9b948a',
  line: '#e8e1d7',
  lineStrong: '#d3c9bb',
  accent: '#7a6a58',
  accentHover: '#665849',
  accentPress: '#52463a',
  accentSoft: '#f6f1ea',
  accentLine: '#ddd2c4',
  onAccent: '#ffffff',
  danger: '#b23a2c',
  dangerSoft: '#fdeeeb',
  warn: '#7e5a12',
  warnSoft: '#fbf4e4',
  warnLine: '#e8d7a9',
  ok: '#256b4c',
  okSoft: '#eef5f0',
};

const graphiteDark: Palette = {
  canvas: '#171512',
  sidebar: '#1d1a16',
  surface: '#221e1a',
  surfaceAlt: '#272320',
  hover: '#302b26',
  active: '#3a342e',
  ink: '#f3efe9',
  muted: '#ada49a',
  faint: '#847c72',
  line: '#2e2924',
  lineStrong: '#413a33',
  accent: '#cbb69b',
  accentHover: '#dcc9b0',
  accentPress: '#b8a288',
  accentSoft: '#2a251f',
  accentLine: '#463c30',
  onAccent: '#14110d',
  danger: '#ea8b7b',
  dangerSoft: '#31201c',
  warn: '#d7ab63',
  warnSoft: '#2a2417',
  warnLine: '#463a1d',
  ok: '#6fc39a',
  okSoft: '#16281f',
};

const duskLight: Palette = {
  canvas: '#ffffff',
  sidebar: '#f8f7fc',
  surface: '#ffffff',
  surfaceAlt: '#f4f2fa',
  hover: '#f0edf8',
  active: '#e8e3f4',
  ink: '#191725',
  muted: '#615c73',
  faint: '#968fa8',
  line: '#e7e3f0',
  lineStrong: '#d2cbe2',
  accent: '#7c5cd6',
  accentHover: '#6a4bc2',
  accentPress: '#573ca3',
  accentSoft: '#f2eefc',
  accentLine: '#dccff5',
  onAccent: '#ffffff',
  danger: '#b83a4b',
  dangerSoft: '#fdeef1',
  warn: '#7d5a12',
  warnSoft: '#fbf5e5',
  warnLine: '#e8d8ab',
  ok: '#256b56',
  okSoft: '#eef5f1',
};

const duskDark: Palette = {
  canvas: '#141318',
  sidebar: '#191821',
  surface: '#1e1c27',
  surfaceAlt: '#23212d',
  hover: '#2b2837',
  active: '#343040',
  ink: '#f1eff7',
  muted: '#a7a2b6',
  faint: '#7f7a8f',
  line: '#2a2833',
  lineStrong: '#3d394a',
  accent: '#b9a3f7',
  accentHover: '#cbb8ff',
  accentPress: '#a68ce8',
  accentSoft: '#241f33',
  accentLine: '#3c3357',
  onAccent: '#120f1a',
  danger: '#ef8b95',
  dangerSoft: '#322026',
  warn: '#d9ac63',
  warnSoft: '#2a2417',
  warnLine: '#463a1d',
  ok: '#6dc3a4',
  okSoft: '#16281f',
};

const BUILTIN: ThemeManifest[] = [
  {
    format: THEME_FORMAT,
    schemaVersion: THEME_SCHEMA_VERSION,
    id: 'dsh',
    name: '静默蓝',
    version: '1.0.0',
    author: '同频',
    description: '默认主题：浅灰侧栏与白画布，蓝色只留给主操作与聚焦。',
    light: dshLight,
    dark: dshDark,
    style: { radius: 12, fontSize: 14 },
  },
  {
    format: THEME_FORMAT,
    schemaVersion: THEME_SCHEMA_VERSION,
    id: 'forest',
    name: '林间绿',
    version: '1.0.0',
    author: '同频',
    description: '低饱和的绿灰中性色，适合长时间对着聊天窗口。',
    light: forestLight,
    dark: forestDark,
    style: { radius: 10, fontSize: 14 },
  },
  {
    format: THEME_FORMAT,
    schemaVersion: THEME_SCHEMA_VERSION,
    id: 'graphite',
    name: '暖石墨',
    version: '1.0.0',
    author: '同频',
    description: '偏暖的灰纸质感，强调色取自主色系的暖褐。',
    light: graphiteLight,
    dark: graphiteDark,
    style: { radius: 8, fontSize: 14 },
  },
  {
    format: THEME_FORMAT,
    schemaVersion: THEME_SCHEMA_VERSION,
    id: 'dusk',
    name: '暮紫',
    version: '1.0.0',
    author: '同频',
    description: '带一点紫调的中性界面，深色模式偏夜色。',
    light: duskLight,
    dark: duskDark,
    style: { radius: 14, fontSize: 14 },
  },
];

export const BUILTIN_THEME_IDS: readonly string[] = BUILTIN.map(theme => theme.id);

/* ---------- validation ---------- */

const HEX = /^#[0-9a-fA-F]{6}$/;
const ID_PATTERN = /^[a-z0-9][a-z0-9._-]{2,47}$/;
const VERSION_PATTERN = /^[0-9]{1,3}(\.[0-9]{1,3}){0,2}$/;
const FORBIDDEN_KEYS = new Set(['__proto__', 'constructor', 'prototype']);
const MANIFEST_KEYS = new Set([
  'format',
  'schemaVersion',
  'id',
  'name',
  'version',
  'author',
  'description',
  'light',
  'dark',
  'style',
]);
const STYLE_KEYS = new Set(['radius', 'fontSize']);

const isRecord = (value: unknown): value is Record<string, unknown> =>
  typeof value === 'object' && value !== null && !Array.isArray(value);

/** Reject prototype-pollution keys anywhere in the parsed document. */
function findForbiddenKey(value: unknown, path: string): string | null {
  if (Array.isArray(value)) {
    for (let index = 0; index < value.length; index += 1) {
      const found = findForbiddenKey(value[index], `${path}[${index}]`);
      if (found) return found;
    }
    return null;
  }
  if (!isRecord(value)) return null;
  for (const key of Object.keys(value)) {
    if (FORBIDDEN_KEYS.has(key)) return path ? `${path}.${key}` : key;
    const found = findForbiddenKey(value[key], path ? `${path}.${key}` : key);
    if (found) return found;
  }
  return null;
}

function readPalette(
  source: unknown,
  where: 'light' | 'dark',
  fallback: Palette,
): { ok: true; palette: Palette } | { ok: false; error: string } {
  if (!isRecord(source)) {
    return { ok: false, error: `主题包缺少 ${where} 配色组。` };
  }
  for (const key of Object.keys(source)) {
    if (!(TOKEN_KEYS as readonly string[]).includes(key)) {
      return { ok: false, error: `未知的颜色项：${where}.${key}，请只使用文档中列出语义颜色。` };
    }
  }
  /* Only whitelisted tokens are copied, missing ones inherit the built-in
   * palette so hand-written packs stay small and forward compatible. */
  const palette = {} as Palette;
  for (const token of TOKEN_KEYS) {
    const raw = source[token];
    if (raw === undefined) {
      palette[token] = fallback[token];
      continue;
    }
    if (typeof raw !== 'string' || !HEX.test(raw)) {
      return { ok: false, error: `颜色值无效：${where}.${token} 需要 #RRGGBB 形式的十六进制颜色。` };
    }
    palette[token] = raw.toLowerCase();
  }
  return { ok: true, palette };
}

function readText(
  value: unknown,
  field: string,
  max: number,
  required: boolean,
): { ok: true; value: string } | { ok: false; error: string } {
  if (value === undefined) {
    if (required) return { ok: false, error: `主题包缺少 ${field}。` };
    return { ok: true, value: '' };
  }
  if (typeof value !== 'string') return { ok: false, error: `${field} 必须是文本。` };
  const text = value.trim();
  if (required && text.length === 0) return { ok: false, error: `${field} 不能为空。` };
  if (text.length > max) return { ok: false, error: `${field} 过长（最多 ${max} 个字符）。` };
  if (/[\u0000-\u001f\u007f]/.test(text)) return { ok: false, error: `${field} 含有控制字符。` };
  return { ok: true, value: text };
}

function readNumber(value: unknown, min: number, max: number, fallback: number): number | null {
  if (value === undefined) return fallback;
  if (typeof value !== 'number' || !Number.isFinite(value)) return null;
  const rounded = Math.round(value);
  if (rounded < min || rounded > max) return null;
  return rounded;
}

/** Parse and fully validate one theme document. Never mutates its input. */
export function parseTheme(raw: unknown): ThemeParseResult {
  let doc: unknown = raw;
  if (typeof raw === 'string') {
    const bytes = new TextEncoder().encode(raw).length;
    if (bytes > MAX_THEME_BYTES) {
      return { ok: false, error: `主题包过大：${Math.round(bytes / 1024)} KiB，上限 64 KiB。` };
    }
    try {
      doc = JSON.parse(raw);
    } catch {
      return { ok: false, error: '不是有效的 JSON，无法读取主题包。' };
    }
  }
  if (!isRecord(doc)) return { ok: false, error: '主题包必须是一个 JSON 对象。' };

  const forbidden = findForbiddenKey(doc, '');
  if (forbidden) return { ok: false, error: `主题包包含不允许的字段：${forbidden}。` };

  if (doc.format !== THEME_FORMAT) {
    return { ok: false, error: '这不像同频主题包：缺少 "format": "twinlink-theme"。' };
  }
  if (doc.schemaVersion !== THEME_SCHEMA_VERSION) {
    const got = typeof doc.schemaVersion === 'number' ? doc.schemaVersion : String(doc.schemaVersion);
    return { ok: false, error: `不支持的主题格式版本 ${got}，当前应用只支持版本 1。` };
  }

  for (const key of Object.keys(doc)) {
    if (!MANIFEST_KEYS.has(key)) return { ok: false, error: `未知的主题字段：${key}。` };
  }

  const id = readText(doc.id, 'id', 48, true);
  if (!id.ok) return id;
  if (!ID_PATTERN.test(id.value)) {
    return {
      ok: false,
      error: '主题标识不合法：需为 3–48 位小写字母、数字、点、短横线或下划线，且以字母或数字开头。',
    };
  }

  const name = readText(doc.name, 'name', 40, true);
  if (!name.ok) return name;

  const version = readText(doc.version, 'version', 20, true);
  if (!version.ok) return version;
  if (!VERSION_PATTERN.test(version.value)) {
    return { ok: false, error: 'version 需要形如 1.0.0 的版本号。' };
  }

  const author = readText(doc.author, 'author', 60, false);
  if (!author.ok) return author;
  const description = readText(doc.description, 'description', 120, false);
  if (!description.ok) return description;

  const light = readPalette(doc.light, 'light', dshLight);
  if (!light.ok) return light;
  const dark = readPalette(doc.dark, 'dark', dshDark);
  if (!dark.ok) return dark;

  let style: ThemeStyle = { radius: 12, fontSize: 14 };
  if (doc.style !== undefined) {
    if (!isRecord(doc.style)) return { ok: false, error: 'style 必须是一个对象。' };
    for (const key of Object.keys(doc.style)) {
      if (!STYLE_KEYS.has(key)) return { ok: false, error: `未知的样式字段：style.${key}。` };
    }
    const radius = readNumber(doc.style.radius, 0, 18, 12);
    if (radius === null) return { ok: false, error: 'style.radius 需要是 0–18 之间的整数。' };
    const fontSize = readNumber(doc.style.fontSize, 12, 18, 14);
    if (fontSize === null) return { ok: false, error: 'style.fontSize 需要是 12–18 之间的整数。' };
    style = { radius, fontSize };
  }

  return {
    ok: true,
    theme: {
      format: THEME_FORMAT,
      schemaVersion: THEME_SCHEMA_VERSION,
      id: id.value,
      name: name.value,
      version: version.value,
      ...(author.value ? { author: author.value } : {}),
      ...(description.value ? { description: description.value } : {}),
      light: light.palette,
      dark: dark.palette,
      style,
    },
  };
}

/* ---------- colour maths (custom accent protection) ---------- */

const clampByte = (value: number) => Math.max(0, Math.min(255, Math.round(value)));

function hexToRgb(hex: string): [number, number, number] {
  const clean = hex.replace('#', '');
  return [
    parseInt(clean.slice(0, 2), 16),
    parseInt(clean.slice(2, 4), 16),
    parseInt(clean.slice(4, 6), 16),
  ];
}

function rgbToHex(rgb: [number, number, number]): string {
  return `#${rgb.map(value => clampByte(value).toString(16).padStart(2, '0')).join('')}`;
}

function mix(a: string, b: string, weight: number): string {
  const [ar, ag, ab] = hexToRgb(a);
  const [br, bg, bb] = hexToRgb(b);
  return rgbToHex([
    ar + (br - ar) * weight,
    ag + (bg - ag) * weight,
    ab + (bb - ab) * weight,
  ]);
}

function luminance(hex: string): number {
  const channel = (value: number) => {
    const c = value / 255;
    return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  };
  const [r, g, b] = hexToRgb(hex);
  return 0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b);
}

function contrast(a: string, b: string): number {
  const la = luminance(a);
  const lb = luminance(b);
  const [hi, lo] = la > lb ? [la, lb] : [lb, la];
  return (hi + 0.05) / (lo + 0.05);
}

/** Nudge a custom accent until it is visible against the canvas. */
function readableAccent(accent: string, canvas: string, ink: string): string {
  let value = accent;
  for (let step = 0; step < 12 && contrast(value, canvas) < 2.4; step += 1) {
    value = mix(value, ink, 0.12);
  }
  return value;
}

function accentFamily(accent: string, mode: ResolvedMode, canvas: string): Pick<
  Palette,
  'accent' | 'accentHover' | 'accentPress' | 'accentSoft' | 'accentLine' | 'onAccent'
> {
  const towards = mode === 'light' ? '#000000' : '#ffffff';
  const onWhite = contrast('#ffffff', accent);
  const onInk = contrast('#101318', accent);
  return {
    accent,
    accentHover: mix(accent, towards, 0.14),
    accentPress: mix(accent, towards, 0.26),
    accentSoft: mix(accent, canvas, mode === 'light' ? 0.92 : 0.86),
    accentLine: mix(accent, canvas, mode === 'light' ? 0.62 : 0.52),
    /* Pick whichever foreground actually reads on the fill. */
    onAccent: onWhite >= onInk ? '#ffffff' : '#101318',
  };
}

/* ---------- state ---------- */

interface StoredState {
  version: number;
  mode: ThemeMode;
  themeId: string;
  accent: string | null;
  fontSize: number | null;
  themes: ThemeManifest[];
}

const systemQuery = (): MediaQueryList | null =>
  typeof window !== 'undefined' && typeof window.matchMedia === 'function'
    ? window.matchMedia('(prefers-color-scheme: dark)')
    : null;

function systemMode(): ResolvedMode {
  return systemQuery()?.matches ? 'dark' : 'light';
}

const defaultState = (): StoredState => ({
  version: STORAGE_VERSION,
  mode: 'system',
  themeId: DEFAULT_THEME_ID,
  accent: null,
  fontSize: null,
  themes: [],
});

function readStorage(): StoredState {
  const fallback = defaultState();
  let text: string | null = null;
  try {
    text = window.localStorage.getItem(STORAGE_KEY);
  } catch {
    return fallback;
  }
  if (!text) return fallback;
  let parsed: unknown;
  try {
    parsed = JSON.parse(text);
  } catch {
    return fallback;
  }
  if (!isRecord(parsed) || parsed.version !== STORAGE_VERSION) return fallback;

  const mode: ThemeMode =
    parsed.mode === 'light' || parsed.mode === 'dark' || parsed.mode === 'system'
      ? parsed.mode
      : 'system';

  const themes: ThemeManifest[] = [];
  if (Array.isArray(parsed.themes)) {
    for (const entry of parsed.themes.slice(0, MAX_USER_THEMES * 2)) {
      const result = parseTheme(entry);
      if (!result.ok) continue;
      if (BUILTIN_THEME_IDS.includes(result.theme.id)) continue;
      if (themes.some(theme => theme.id === result.theme.id)) continue;
      themes.push(result.theme);
    }
  }

  const known = new Set([...BUILTIN_THEME_IDS, ...themes.map(theme => theme.id)]);
  const themeId =
    typeof parsed.themeId === 'string' && known.has(parsed.themeId) ? parsed.themeId : DEFAULT_THEME_ID;

  const accent =
    typeof parsed.accent === 'string' && HEX.test(parsed.accent) ? parsed.accent.toLowerCase() : null;

  const fontSize =
    typeof parsed.fontSize === 'number' &&
    Number.isInteger(parsed.fontSize) &&
    parsed.fontSize >= 12 &&
    parsed.fontSize <= 18
      ? parsed.fontSize
      : null;

  return { version: STORAGE_VERSION, mode, themeId, accent, fontSize, themes };
}

let state: StoredState = defaultState();
let initialized = false;
const listeners = new Set<() => void>();

function persist(): boolean {
  try {
    window.localStorage.setItem(STORAGE_KEY, JSON.stringify(state));
    return true;
  } catch {
    return false;
  }
}

function emit(): void {
  for (const listener of listeners) {
    try {
      listener();
    } catch {
      /* A broken listener must not break the theme runtime. */
    }
  }
}

/* ---------- theme lookup ---------- */

function findTheme(id: string): ThemeManifest | undefined {
  return BUILTIN.find(theme => theme.id === id) ?? state.themes.find(theme => theme.id === id);
}

export function isBuiltinTheme(id: string): boolean {
  return BUILTIN_THEME_IDS.includes(id);
}

export function getActiveTheme(): ThemeManifest {
  return findTheme(state.themeId) ?? BUILTIN[0];
}

export function resolveMode(): ResolvedMode {
  if (state.mode === 'system') return systemMode();
  return state.mode;
}

export function getAppearance(): AppearanceState {
  return {
    mode: state.mode,
    resolved: resolveMode(),
    themeId: getActiveTheme().id,
    accent: state.accent,
    fontSize: state.fontSize,
  };
}

export function listThemes(): ThemeSummary[] {
  const toSummary = (theme: ThemeManifest, builtin: boolean): ThemeSummary => ({
    id: theme.id,
    name: theme.name,
    version: theme.version,
    author: theme.author ?? '',
    description: theme.description ?? '',
    builtin,
    style: theme.style,
    light: theme.light,
    dark: theme.dark,
  });
  return [
    ...BUILTIN.map(theme => toSummary(theme, true)),
    ...state.themes.map(theme => toSummary(theme, false)),
  ];
}

export function getTheme(id: string): ThemeManifest | undefined {
  return findTheme(id);
}

/* ---------- applying ---------- */

/** Active palette for one colour scheme, with the user accent layered on top. */
export function paletteFor(mode: ResolvedMode, theme = getActiveTheme()): Palette {
  const base = mode === 'dark' ? theme.dark : theme.light;
  if (!state.accent) return base;
  const accent = readableAccent(state.accent, base.canvas, base.ink);
  return { ...base, ...accentFamily(accent, mode, base.canvas) };
}

const cssName = (token: string) => `--tl-${token.replace(/[A-Z]/g, char => `-${char.toLowerCase()}`)}`;

function apply(): void {
  if (typeof document === 'undefined') return;
  const root = document.documentElement;
  const resolved = resolveMode();
  const theme = getActiveTheme();
  const palette = paletteFor(resolved, theme);

  root.dataset.theme = resolved;
  root.dataset.themeId = theme.id;
  root.style.colorScheme = resolved;

  for (const token of TOKEN_KEYS) {
    root.style.setProperty(cssName(token), palette[token]);
  }

  /* Kept for the few places that still read a generic accent name. */
  root.style.setProperty('--accent', palette.accent);
  root.style.setProperty('--accent-contrast', palette.onAccent);

  const fontSize = state.fontSize ?? theme.style.fontSize;
  root.style.fontSize = `${fontSize}px`;
  root.style.setProperty('--tl-font-size', `${fontSize}px`);

  const radius = theme.style.radius;
  root.style.setProperty('--tl-radius', `${radius}px`);
  root.style.setProperty('--tl-radius-sm', `${Math.max(0, radius - 4)}px`);
  root.style.setProperty('--tl-radius-lg', `${Math.min(20, radius + 4)}px`);

  const meta = document.querySelector('meta[name="color-scheme"]');
  if (meta) meta.setAttribute('content', resolved === 'dark' ? 'dark light' : 'light dark');
}

/* ---------- public mutations ---------- */

export function setMode(mode: ThemeMode): void {
  if (mode !== 'light' && mode !== 'dark' && mode !== 'system') return;
  state.mode = mode;
  persist();
  apply();
  emit();
}

export function setTheme(themeId: string): boolean {
  if (!findTheme(themeId)) return false;
  state.themeId = themeId;
  /* Switching themes drops a stale accent override only when the user never
   * picked one; an explicit custom accent stays until reset. */
  persist();
  apply();
  emit();
  return true;
}

export function setAccent(accent: string | null): boolean {
  if (accent !== null && !HEX.test(accent)) return false;
  state.accent = accent === null ? null : accent.toLowerCase();
  persist();
  apply();
  emit();
  return true;
}

export function setFontSize(size: number | null): boolean {
  if (size !== null) {
    if (!Number.isFinite(size)) return false;
    const rounded = Math.round(size);
    if (rounded < 12 || rounded > 18) return false;
    state.fontSize = rounded;
  } else {
    state.fontSize = null;
  }
  persist();
  apply();
  emit();
  return true;
}

/** Back to the built-in default theme, system mode, no overrides. */
export function resetAppearance(): void {
  state = { ...defaultState(), themes: state.themes };
  persist();
  apply();
  emit();
}

export function subscribeTheme(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

/* ---------- plugin management ---------- */

/** Remove a user theme. Uninstalling the active theme falls back to default. */
export function uninstallTheme(id: string): { ok: true; fellBack: boolean } | { ok: false; error: string } {
  if (isBuiltinTheme(id)) return { ok: false, error: '内置主题不能删除。' };
  const index = state.themes.findIndex(theme => theme.id === id);
  if (index < 0) return { ok: false, error: '没有找到这个主题。' };
  const wasActive = state.themeId === id;
  const themes = state.themes.slice();
  themes.splice(index, 1);
  const previous = { ...state };
  state = { ...state, themes, themeId: wasActive ? DEFAULT_THEME_ID : state.themeId };
  if (!persist()) {
    state = previous;
    return { ok: false, error: '本地存储不可用，主题未被删除。' };
  }
  apply();
  emit();
  return { ok: true, fellBack: wasActive };
}

export function installTheme(raw: unknown, options: { replace?: boolean } = {}): ThemeInstallResult {
  const parsed = parseTheme(raw);
  if (!parsed.ok) return { ok: false, error: parsed.error };
  const theme = parsed.theme;

  if (isBuiltinTheme(theme.id)) {
    return { ok: false, error: `主题标识 "${theme.id}" 属于内置主题，请换一个标识再导入。` };
  }
  const existing = state.themes.find(candidate => candidate.id === theme.id);
  if (existing && !options.replace) {
    return {
      ok: false,
      conflict: theme,
      error: `已安装同名主题「${existing.name}」（v${existing.version}），确认覆盖后才会替换。`,
    };
  }
  if (!existing && state.themes.length >= MAX_USER_THEMES) {
    return { ok: false, error: `最多安装 ${MAX_USER_THEMES} 个主题，请先删除不用的主题。` };
  }

  const previous = { ...state, themes: state.themes.slice() };
  const themes = state.themes.slice();
  const index = themes.findIndex(candidate => candidate.id === theme.id);
  if (index >= 0) themes[index] = theme;
  else themes.push(theme);

  state = { ...state, themes };
  if (!persist()) {
    state = previous;
    return { ok: false, error: '本地存储空间不足，主题没有安装。' };
  }
  /* Replacing the active theme repaints immediately, without a manual re-select. */
  if (state.themeId === theme.id) apply();
  emit();
  return { ok: true, theme, replaced: index >= 0 };
}

/** Serialise the effective theme as a re-installable pack. */
export function exportThemeJson(themeId?: string): string {
  const source = findTheme(themeId ?? state.themeId) ?? BUILTIN[0];
  const builtin = isBuiltinTheme(source.id);
  const id = builtin ? `exported-${source.id}`.slice(0, 48) : source.id;
  const theme: ThemeManifest = {
    format: THEME_FORMAT,
    schemaVersion: THEME_SCHEMA_VERSION,
    id,
    name: builtin ? `${source.name}·我的版本` : source.name,
    version: source.version,
    author: source.author ?? '同频用户',
    description: source.description,
    light: { ...source.light },
    dark: { ...source.dark },
    style: { ...source.style },
  };
  /* The effective font size and accent travel with the exported pack. */
  if (state.fontSize !== null) theme.style = { ...theme.style, fontSize: state.fontSize };
  if (state.accent) {
    const family = accentFamily(state.accent, 'light', theme.light.canvas);
    const familyDark = accentFamily(state.accent, 'dark', theme.dark.canvas);
    theme.light = { ...theme.light, ...family };
    theme.dark = { ...theme.dark, ...familyDark };
  }
  return JSON.stringify(theme, null, 2);
}

/* ---------- bootstrap ---------- */

/**
 * Read stored appearance and paint it. Safe to call once, early in the module
 * graph, so the first frame already uses the right colour scheme.
 */
export function initTheme(): void {
  if (initialized || typeof window === 'undefined') return;
  initialized = true;
  state = readStorage();
  apply();

  const query = systemQuery();
  if (query) {
    const onChange = () => {
      if (state.mode !== 'system') return;
      apply();
      emit();
    };
    if (typeof query.addEventListener === 'function') query.addEventListener('change', onChange);
    else if (typeof query.addListener === 'function') query.addListener(onChange);
  }

  /* Another window (or a dev refresh in a second tab) edited the appearance. */
  window.addEventListener('storage', event => {
    if (event.key !== STORAGE_KEY) return;
    state = readStorage();
    apply();
    emit();
  });
}
