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
  if (!value) return { palette: 'forge', mode: 'dark' };

  if (value === 'dark' || value === 'light') {
    return { palette: 'forge', mode: value };
  }

  const [palette, mode] = value.split('-') as [ThemePaletteId | undefined, ThemeMode | undefined];
  if (palette && paletteIds.has(palette) && (mode === 'dark' || mode === 'light')) {
    return { palette, mode };
  }

  return { palette: 'forge', mode: 'dark' };
}

export function oppositeMode(mode: ThemeMode): ThemeMode {
  return mode === 'dark' ? 'light' : 'dark';
}
