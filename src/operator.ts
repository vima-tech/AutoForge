/**
 * 操作者身份卡的全局轻量 store。
 *
 * MeAvatar 散落在 rail 与会议室多处，需要在身份卡编辑后即时统一刷新。
 * 这里用「模块单例 cache + 订阅」模式（与 App.tsx 的 badges-refresh 同构），
 * 避免给整个组件树套 Context Provider。真源仍是后端 app_settings.operator_profile。
 */
import { useEffect, useState } from 'react';
import { getOperatorProfile, setOperatorProfile, type OperatorProfile } from './services';

export type { OperatorProfile };

export const DEFAULT_OPERATOR: OperatorProfile = {
  display_name: '我',
  avatar: '管',
  accent_color: '',
  role: '操作者',
};

let cache: OperatorProfile = DEFAULT_OPERATOR;
let loaded = false;
const subscribers = new Set<() => void>();

function notify() {
  subscribers.forEach(fn => fn());
}

export function getOperator(): OperatorProfile {
  return cache;
}

/** 从后端拉取一次（应用启动时调用）。失败时静默保留默认值。 */
export async function loadOperator(): Promise<void> {
  try {
    cache = await getOperatorProfile();
    loaded = true;
    notify();
  } catch {
    /* 浏览器模式 / 后端未就绪：保留默认值 */
  }
}

/** 保存到后端并就地更新 cache（返回归一化后的 profile）。 */
export async function saveOperator(next: OperatorProfile): Promise<OperatorProfile> {
  const saved = await setOperatorProfile(next);
  cache = saved;
  loaded = true;
  notify();
  return saved;
}

/** 订阅式 hook：身份卡任意一处变更后，所有用到的组件一起重渲染。 */
export function useOperator(): OperatorProfile {
  const [, force] = useState(0);
  useEffect(() => {
    const fn = () => force(x => x + 1);
    subscribers.add(fn);
    if (!loaded) void loadOperator();
    return () => { subscribers.delete(fn); };
  }, []);
  return cache;
}
