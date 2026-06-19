export type ThemePaletteId =
  | 'forge'
  | 'carbon'
  | 'moss'
  | 'harbor'
  | 'rose'
  | 'indigo'
  | 'volt'
  | 'copper'
  | 'glacier'
  | 'plum'
  | 'saffron'
  | 'abyss';
export type ThemeMode = 'dark' | 'light';
export type ThemeId = `${ThemePaletteId}-${ThemeMode}`;

export interface ThemePalette {
  id: ThemePaletteId;
  name: string;
  subtitle: string;
  accent: string;
  swatches: string[];
}

export interface ThemeSelection {
  palette: ThemePaletteId;
  mode: ThemeMode;
}

export const THEME_STORAGE_KEY = 'AutoForge:theme';

export const THEME_PALETTES: ThemePalette[] = [
  {
    id: 'forge',
    name: 'Forge Ember',
    subtitle: '当前默认配色，温暖熔炉橙',
    accent: '#e8772e',
    swatches: ['#16110d', '#e8772e', '#f1e8d9'],
  },
  {
    id: 'carbon',
    name: 'Carbon Cyan',
    subtitle: '冷静碳黑底，青色操作焦点',
    accent: '#2f9ea3',
    swatches: ['#101416', '#2f9ea3', '#e7f0ee'],
  },
  {
    id: 'moss',
    name: 'Moss Field',
    subtitle: '苔绿和石墨灰，适合长时间工作',
    accent: '#5f8f5f',
    swatches: ['#11170f', '#5f8f5f', '#edf2e7'],
  },
  {
    id: 'harbor',
    name: 'Harbor Steel',
    subtitle: '港湾蓝灰，强调审计和数据扫描',
    accent: '#477fb3',
    swatches: ['#10151c', '#477fb3', '#e8eef5'],
  },
  {
    id: 'rose',
    name: 'Rose Alloy',
    subtitle: '低饱和玫瑰金属，不刺眼',
    accent: '#b8616f',
    swatches: ['#181214', '#b8616f', '#f3e9ea'],
  },
  {
    id: 'indigo',
    name: 'Ink Indigo',
    subtitle: '墨蓝和珊瑚点缀，夜间层次更硬朗',
    accent: '#697bb8',
    swatches: ['#111320', '#697bb8', '#eceefa'],
  },
  {
    id: 'volt',
    name: 'Volt Matrix',
    subtitle: '高对比石墨底，荧黄绿强调执行状态',
    accent: '#b5c84b',
    swatches: ['#10120e', '#b5c84b', '#f0f2df'],
  },
  {
    id: 'copper',
    name: 'Copper Slate',
    subtitle: '冷石板和氧化铜，偏工业控制台',
    accent: '#b66b4d',
    swatches: ['#151413', '#b66b4d', '#f1ece6'],
  },
  {
    id: 'glacier',
    name: 'Glacier Mint',
    subtitle: '冰川灰白和薄荷蓝，清爽低压',
    accent: '#4aaca1',
    swatches: ['#0f1718', '#4aaca1', '#eef7f5'],
  },
  {
    id: 'plum',
    name: 'Plum Graphite',
    subtitle: '石墨底配深李子色，冷静但更有张力',
    accent: '#9b5f8c',
    swatches: ['#171318', '#9b5f8c', '#f1e8ef'],
  },
  {
    id: 'saffron',
    name: 'Saffron Signal',
    subtitle: '暗钢灰和藏红黄，告警与行动感更强',
    accent: '#c4932f',
    swatches: ['#15140f', '#c4932f', '#f4efd9'],
  },
  {
    id: 'abyss',
    name: 'Abyss Coral',
    subtitle: '深海近黑底，珊瑚色作为唯一热源',
    accent: '#d46f5f',
    swatches: ['#0f1518', '#d46f5f', '#eef2f3'],
  },
];

const paletteIds = new Set<ThemePaletteId>(THEME_PALETTES.map(p => p.id));

export function themeIdOf(selection: ThemeSelection): ThemeId {
  return `${selection.palette}-${selection.mode}`;
}

export function parseTheme(value: string | null | undefined): ThemeSelection {
  if (!value) return { palette: 'forge', mode: 'light' };

  if (value === 'dark' || value === 'light') {
    return { palette: 'forge', mode: value };
  }

  const [palette, mode] = value.split('-') as [ThemePaletteId | undefined, ThemeMode | undefined];
  if (palette && paletteIds.has(palette) && (mode === 'dark' || mode === 'light')) {
    return { palette, mode };
  }

  return { palette: 'forge', mode: 'light' };
}

export function oppositeMode(mode: ThemeMode): ThemeMode {
  return mode === 'dark' ? 'light' : 'dark';
}

// Nav rail expand behavior:
//   'hover'  — rail expands to show labels on hover (default)
//   'locked' — rail is locked collapsed; hover never triggers the expanded state
export type RailMode = 'hover' | 'locked';

export const RAIL_STORAGE_KEY = 'AutoForge:railMode';

export function parseRailMode(value: string | null | undefined): RailMode {
  return value === 'locked' ? 'locked' : 'hover';
}

export function applyRailMode(mode: RailMode): void {
  document.documentElement.setAttribute('data-rail', mode);
}

// ---- 速录念头全局快捷键 ----
// 组合键基于 KeyboardEvent.code（与键盘布局无关的物理键，如 'KeyN' / 'Space'），
// 配合四个修饰键标志。存为 JSON 落 localStorage，沿用主题/导航栏的纯前端偏好模式。
export interface ShortcutCombo {
  ctrl: boolean;
  meta: boolean;
  alt: boolean;
  shift: boolean;
  code: string;
}

export interface QuickCaptureShortcut {
  enabled: boolean;
  combo: ShortcutCombo;
}

export const QUICK_CAPTURE_SHORTCUT_KEY = 'AutoForge:quickCaptureShortcut';
// 快速语音录入快捷键：在任意含语音录入的界面（会议室 Composer、速录念头…）切换录音。
export const VOICE_INPUT_SHORTCUT_KEY = 'AutoForge:voiceInputShortcut';
// 配置变更后在 window 上派发，让 App 实时重读（无需刷新即生效）。
export const SHORTCUT_CHANGED_EVENT = 'AutoForge:shortcut-changed';

const isMac = typeof navigator !== 'undefined' && /mac/i.test(navigator.platform);

export const DEFAULT_QUICK_CAPTURE_SHORTCUT: QuickCaptureShortcut = {
  enabled: true,
  // macOS 上是 ⌥N，其余平台 Alt+N —— 避开 Ctrl/⌘+Shift+I/J/C 等 devtools 组合。
  combo: { ctrl: false, meta: false, alt: true, shift: false, code: 'KeyN' },
};

export const DEFAULT_VOICE_INPUT_SHORTCUT: QuickCaptureShortcut = {
  enabled: true,
  // macOS 上是 ⌥M，其余平台 Alt+M（M = mic）—— 避开速录的 Alt+N 与 devtools 组合。
  combo: { ctrl: false, meta: false, alt: true, shift: false, code: 'KeyM' },
};

// 通用快捷键反序列化：JSON 解析失败或缺主键时回退到给定默认值。
export function parseShortcut(value: string | null | undefined, fallback: QuickCaptureShortcut): QuickCaptureShortcut {
  if (!value) return fallback;
  try {
    const raw = JSON.parse(value) as { enabled?: unknown; combo?: Partial<ShortcutCombo> };
    const c = raw.combo;
    if (!c || typeof c.code !== 'string' || !c.code) return fallback;
    return {
      enabled: raw.enabled !== false,
      combo: { ctrl: !!c.ctrl, meta: !!c.meta, alt: !!c.alt, shift: !!c.shift, code: c.code },
    };
  } catch {
    return fallback;
  }
}

export function parseQuickCaptureShortcut(value: string | null | undefined): QuickCaptureShortcut {
  return parseShortcut(value, DEFAULT_QUICK_CAPTURE_SHORTCUT);
}

export function parseVoiceInputShortcut(value: string | null | undefined): QuickCaptureShortcut {
  return parseShortcut(value, DEFAULT_VOICE_INPUT_SHORTCUT);
}

// 该键本身只是修饰键（尚未按下主键时不应记录组合）。
export function isModifierCode(code: string): boolean {
  return /^(Control|Shift|Alt|Meta)(Left|Right)$/.test(code);
}

export function comboHasModifier(combo: ShortcutCombo): boolean {
  return combo.ctrl || combo.meta || combo.alt || combo.shift;
}

function codeLabel(code: string): string {
  if (code.startsWith('Key')) return code.slice(3);
  if (code.startsWith('Digit')) return code.slice(5);
  if (code.startsWith('Numpad')) return 'Num' + code.slice(6);
  const map: Record<string, string> = {
    Space: 'Space', Enter: 'Enter', Tab: 'Tab', Backquote: '`',
    Minus: '-', Equal: '=', BracketLeft: '[', BracketRight: ']',
    Semicolon: ';', Quote: "'", Comma: ',', Period: '.', Slash: '/', Backslash: '\\',
    ArrowUp: '↑', ArrowDown: '↓', ArrowLeft: '←', ArrowRight: '→',
  };
  return map[code] ?? code;
}

export function formatCombo(combo: ShortcutCombo): string {
  const parts: string[] = [];
  if (combo.ctrl) parts.push(isMac ? '⌃' : 'Ctrl');
  if (combo.alt) parts.push(isMac ? '⌥' : 'Alt');
  if (combo.shift) parts.push(isMac ? '⇧' : 'Shift');
  if (combo.meta) parts.push(isMac ? '⌘' : 'Meta');
  parts.push(codeLabel(combo.code));
  return parts.join(isMac ? ' ' : ' + ');
}

export function comboMatchesEvent(combo: ShortcutCombo, e: KeyboardEvent): boolean {
  return (
    e.ctrlKey === combo.ctrl &&
    e.metaKey === combo.meta &&
    e.altKey === combo.alt &&
    e.shiftKey === combo.shift &&
    e.code === combo.code
  );
}
