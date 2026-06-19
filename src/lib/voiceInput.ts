/**
 * 全局快速语音录入的「活跃录音面」登记表。
 *
 * 含语音录入的界面（会议室 Composer、速录念头…）在挂载时把自己的「切换录音」回调
 * 压栈、卸载时出栈。全局语音快捷键触发时调用栈顶（最近挂载、即视觉最上层）那个面的
 * 回调，从而实现「一个快捷键在任意语音界面开/关录音」。栈为空说明当前没有任何语音界面，
 * 由调用方（App）兜底——弹出「速录念头」并自动起录。
 *
 * 不走 React context / 自定义事件，是因为速录念头是 createPortal 挂在 .os-window 上的
 * 独立子树、且要在「没有任何语音界面」时才兜底开速录，用一个简单的模块级栈最直接。
 */
type VoiceToggle = () => void;

const stack: VoiceToggle[] = [];

/** 登记一个语音录入面，返回注销函数（在组件卸载时调用）。 */
export function registerVoiceSurface(toggle: VoiceToggle): () => void {
  stack.push(toggle);
  return () => {
    const i = stack.lastIndexOf(toggle);
    if (i >= 0) stack.splice(i, 1);
  };
}

/**
 * 触发快速语音录入：切换栈顶语音面的录音状态。
 * @returns 有活跃语音面并已处理返回 true；栈为空返回 false（由调用方兜底）。
 */
export function triggerVoiceInput(): boolean {
  const top = stack[stack.length - 1];
  if (!top) return false;
  top();
  return true;
}
