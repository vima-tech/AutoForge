/**
 * Agent 列表的全局轻量 store（DB 真源）。
 *
 * Markdown 的 @提及高亮、Avatar 的 id→Agent 查表都需要「当前真实 Agent 列表」，
 * 但二者散落在消息块深处（Block → Markdown）、无法方便地 prop 透传。
 * 这里复用与 `operator.ts` 同构的「模块单例 cache + 订阅」模式：
 * 组件用 `useAgents()` 订阅，列表异步到位后自动重渲染；真源是后端 `agents` 表
 * （`list_agents` IPC，见记忆 [[project_agent_two_layer]]）。
 *
 * 这取代了旧的 `data/mock.ts` 写死 5 个 Agent（AGENTS/AGENT_MAP）的渲染路径——
 * 用户在设置里自建的 Agent，其 @提及与头像现在都能正确识别。
 */
import { useEffect, useState } from 'react';
import { listAgents, type Agent } from './services';

export type { Agent };

let cache: Agent[] = [];
let cacheMap: Record<string, Agent> = {};
let loaded = false;          // setCache 成功后置真：useAgents 据此决定是否触发懒加载
let lazyRequested = false;    // 懒加载只发一次，避免每个 useAgents 都各拉一遍
let chain: Promise<void> = Promise.resolve();  // 串行队列尾，保证刷新顺序与最终一致
const subscribers = new Set<() => void>();

function notify() {
  subscribers.forEach(fn => fn());
}

function setCache(list: Agent[]) {
  cache = list;
  cacheMap = Object.fromEntries(list.map(a => [a.id, a]));
  loaded = true;
  notify();
}

function fetchAgents(): Promise<void> {
  return listAgents()
    .then(setCache)
    .catch(() => { /* 浏览器模式 / 后端未就绪：保留现有值 */ });
}

export function getAgents(): Agent[] {
  return cache;
}

export function getAgentMap(): Record<string, Agent> {
  return cacheMap;
}

/** 懒加载：仅首次真正拉取，后续调用复用同一队列，不重复请求。 */
export function loadAgents(): Promise<void> {
  if (lazyRequested) return chain;
  lazyRequested = true;
  chain = chain.then(fetchAgents);
  return chain;
}

/**
 * 强制重新拉取（设置里增删改 Agent 后调用，让渲染层即时同步）。
 * 接到串行队列尾部，保证排在任何在飞行的拉取之后，最终读到 mutation 后的最新列表。
 */
export function refreshAgents(): Promise<void> {
  lazyRequested = true;
  chain = chain.then(fetchAgents);
  return chain;
}

/** 用已加载好的列表就地刷新 cache（避免重复拉取，如会议室已 listAgents）。 */
export function primeAgents(list: Agent[]): void {
  lazyRequested = true;  // 已有真源，无需再触发懒加载
  setCache(list);
}

/** 订阅式 hook：Agent 列表任意一处刷新后，所有用到的组件一起重渲染。 */
export function useAgents(): Agent[] {
  const [, force] = useState(0);
  useEffect(() => {
    const fn = () => force(x => x + 1);
    subscribers.add(fn);
    if (!loaded) void loadAgents();
    return () => { subscribers.delete(fn); };
  }, []);
  return cache;
}
