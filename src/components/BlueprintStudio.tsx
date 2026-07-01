import React, { useEffect, useRef, useState } from 'react';
import Icon from './Icon';
import Markdown from './Markdown';
import { RealtimeAsr } from '../lib/realtimeAsr';
import { registerVoiceSurface } from '../lib/voiceInput';
import {
  startBlueprintDraft, refineBlueprintDraft, patchBlueprintDraft, answerBlueprintQuestion,
  getBlueprintDraft, applyBlueprintDraft, codeBlueprintDraft, listProjectFiles,
  type Project, type BlueprintDraftView, type BlueprintSpec, type BlueprintTask, type ProjectContextFile,
  type BlueprintBackend,
} from '../services';

/**
 * 项目蓝图工作台（全屏双栏）：左对话流 / 右产物预览（PRD · 规格 · 任务）。
 * 草稿是单一真源——AI 修正(refine)与人工手改(patch)都写同一份，切项目/换页不丢；
 * 满意后「写入工作区」= PRD 落 docs、规格落 specs+DB、勾选任务入 triage 池（人审闸口不变）。
 * 作为「蓝图」页的内容铺满父容器（height:100%），项目切换由页面顶部的项目选择器负责。
 * 遵循 DESIGN.md：只引 CSS 变量、seg 替 tab、自定义下拉替原生 select。
 */
const SPEC_CATEGORIES = ['tech_stack', 'architecture', 'coding', 'api', 'testing'];
const TASK_CATEGORIES = ['Feature', 'Bug', 'Refactor', 'Chore'];
const SEVERITIES = ['low', 'medium', 'high'];
const PRD_KEY = '__prd__';

// 起草 / 修正的巡回阶段文案：制造「正在推进」的进度感（复用 ChangeSummaryCard 同款 cs-phase）。
const GEN_PHASES = ['理解大需求…', '起草 PRD 结构…', '拆解技术规格…', '生成任务清单…', '收口校验…'];
const REFINE_PHASES = ['理解你的指令…', '定位要改的部分…', '重写受影响内容…', '校对整体一致性…'];

// 起草态的快速示例：点一下填进大需求框，降低空白页摩擦。
const EXAMPLE_BRIEFS: { label: string; text: string }[] = [
  { label: 'SaaS 工具', text: '一个面向独立开发者的 SaaS：用户提交一句话想法，AI 自动生成 PRD、技术规格与任务清单，并跟踪从需求到上线的进度。' },
  { label: '内部效率工具', text: '一个团队内部工具：把会议纪要自动拆成需求条目、分派负责人、跟踪状态，并在完成时通知。' },
  { label: '微信小程序商城', text: '一个微信小程序电商：商品浏览与搜索、购物车、微信支付、订单管理与售后客服，后台可上下架商品。' },
];

function uid(): string {
  try { return crypto.randomUUID(); } catch { return 'tmp-' + Math.random().toString(36).slice(2); }
}

/**
 * 跨卸载存活的「在途生成」登记表（键=projectId）。
 * 起草/修正是后端命令、会落库；但若生成途中切走 rail 页，组件会卸载、
 * setState 失效，回来时就看不到「正在生成」也接不到结果——状态像丢了。
 * 把 Promise 存在模块级 Map 里，组件重新挂载时按 projectId 重连同一个 Promise，
 * 既能恢复 busy 指示、也能在它 resolve 时落地结果。模块级单例随 SPA 进程存活。
 */
const inflight = new Map<string, Promise<BlueprintDraftView>>();
function trackInflight(key: string, p: Promise<BlueprintDraftView>) {
  inflight.set(key, p);
  void p.catch(() => {}).finally(() => { if (inflight.get(key) === p) inflight.delete(key); });
}

/** 派生展示状态 → 标签 + chip 语义色。 */
export const DISPLAY_STATUS: Record<string, { label: string; chip: string }> = {
  drafting: { label: '梳理中', chip: '' },
  coding: { label: '编码中', chip: 'amber' },
  in_review: { label: '待代码审核', chip: 'blue' },
  implemented: { label: '已实现', chip: 'green' },
  failed: { label: '编码失败', chip: 'red' },
  conflict: { label: '需解冲突', chip: 'red' },
};

export default function BlueprintStudio({ project, draftId, isNew, onBack, onChanged, onOpenAudit, onOpenDelivery }: {
  project: Project; draftId: string | null; isNew: boolean;
  onBack: () => void; onChanged: (newDraftId?: string) => void;
  onOpenAudit?: (projectId: string, issueId: string) => void;
  onOpenDelivery?: (projectId: string, opts?: { stage?: string; draftId?: string }) => void;
}) {
  const [loading, setLoading] = useState(!isNew);
  const [busy, setBusy] = useState(false);
  const [view, setView] = useState<BlueprintDraftView | null>(null);
  const [briefInput, setBriefInput] = useState('');
  const [chatInput, setChatInput] = useState('');
  const [tab, setTab] = useState<'prd' | 'specs' | 'tasks'>('prd');
  const [changed, setChanged] = useState<Set<string>>(new Set());
  const [writeTasklistDoc, setWriteTasklistDoc] = useState(true);
  const [err, setErr] = useState('');
  const [applyMsg, setApplyMsg] = useState('');
  const [editPrd, setEditPrd] = useState(false);
  const [prdDraft, setPrdDraft] = useState('');
  const [inputFocused, setInputFocused] = useState(false);
  const [genPhase, setGenPhase] = useState(0);
  // 分析后端：需求分析专家(LLM) 或 编码 Agent(CLI 读真实仓库)。无仓库时强制走 LLM。
  const [backend, setBackend] = useState<BlueprintBackend>('analysis');
  // 起草 composer：引用项目文件 + 本地附件 + 语音录入
  const [refFiles, setRefFiles] = useState<string[]>([]);
  const [attachFiles, setAttachFiles] = useState<File[]>([]);
  const [filePickerOpen, setFilePickerOpen] = useState(false);
  const [projectFiles, setProjectFiles] = useState<ProjectContextFile[]>([]);
  const [fileFilter, setFileFilter] = useState('');
  const [voiceOn, setVoiceOn] = useState(false);
  const [voiceConnecting, setVoiceConnecting] = useState(false);
  const chatEndRef = useRef<HTMLDivElement>(null);
  const filePickerRef = useRef<HTMLDivElement>(null);
  const attachInputRef = useRef<HTMLInputElement>(null);
  const asrRef = useRef<RealtimeAsr | null>(null);
  const voiceBaseRef = useRef('');
  const voiceCommittedRef = useRef('');
  const voicePartialRef = useRef('');

  const draft = view?.draft ?? null;
  const messages = view?.messages ?? [];
  const coding = draft?.status === 'coding';   // 编码后锁定编辑
  const noRepo = !project.repo_path?.trim();
  const flowKey = draftId ?? `new:${project.id}`;

  // 载入指定草稿（isNew 则跳过，进起草 composer）。
  useEffect(() => {
    if (isNew || !draftId) { setView(null); setLoading(false); return; }
    let alive = true;
    (async () => {
      setLoading(true);
      try {
        const v = await getBlueprintDraft(draftId);
        if (alive) setView(v);
      } catch (e) { if (alive) setErr(String(e)); }
      finally { if (alive) setLoading(false); }
      // 重连在途修正：切走又回来时若该草稿仍在生成，恢复 busy 并在 resolve 时落地。
      const p = inflight.get(draftId);
      if (p && alive) {
        setBusy(true);
        try { const v = await p; if (alive) setView(v); }
        catch (e) { if (alive) setErr(String(e)); }
        finally { if (alive) setBusy(false); }
      }
    })();
    return () => { alive = false; };
  }, [draftId, isNew]);

  useEffect(() => { chatEndRef.current?.scrollIntoView({ behavior: 'smooth' }); }, [messages.length]);

  // 生成期每 1.5s 推进一个阶段文案（停在最后一句，不回卷），传达持续进度。
  useEffect(() => {
    if (!busy) { setGenPhase(0); return; }
    const id = window.setInterval(() => setGenPhase(p => p + 1), 1500);
    return () => window.clearInterval(id);
  }, [busy]);

  // 计算两份草稿间被改/新增的 id 集合，用于预览高亮。
  function diffIds(prev: BlueprintDraftView | null, next: BlueprintDraftView): Set<string> {
    const s = new Set<string>();
    const oldSpecs = new Map((prev?.draft.specs ?? []).map(x => [x.id, JSON.stringify(x)]));
    for (const sp of next.draft.specs) if (oldSpecs.get(sp.id) !== JSON.stringify(sp)) s.add(sp.id);
    const oldTasks = new Map((prev?.draft.tasklist ?? []).map(x => [x.id, JSON.stringify(x)]));
    for (const t of next.draft.tasklist) if (oldTasks.get(t.id) !== JSON.stringify(t)) s.add(t.id);
    if ((prev?.draft.prd_markdown ?? '') !== next.draft.prd_markdown) s.add(PRD_KEY);
    return s;
  }
  function flash(ids: Set<string>) {
    setChanged(ids);
    window.setTimeout(() => setChanged(new Set()), 4200);
  }

  // ── 语音录入（复用 RealtimeAsr，结果实时写入大需求框）+ 全局快捷键登记 ──
  const startVoice = async () => {
    if (voiceOn || voiceConnecting) return;
    setErr(''); setVoiceConnecting(true);
    voiceBaseRef.current = briefInput ? briefInput + (briefInput.endsWith(' ') ? '' : ' ') : '';
    voiceCommittedRef.current = ''; voicePartialRef.current = '';
    const rt = new RealtimeAsr();
    asrRef.current = rt;
    try {
      await rt.start((t, isFinal) => {
        if (isFinal) { voiceCommittedRef.current += t; voicePartialRef.current = ''; }
        else { voicePartialRef.current = t; }
        setBriefInput(voiceBaseRef.current + voiceCommittedRef.current + voicePartialRef.current);
      }, () => { setVoiceConnecting(false); setVoiceOn(true); });
    } catch (e) {
      asrRef.current = null; setVoiceOn(false); setErr(String(e) + '；可改用文字输入');
    } finally { setVoiceConnecting(false); }
  };
  const stopVoice = async () => {
    setVoiceOn(false);
    const rt = asrRef.current; asrRef.current = null;
    await rt?.stop();
  };
  // 全局语音快捷键：登记本面为活跃语音面（仅起草态生效）。
  const voiceToggleRef = useRef<() => void>(() => {});
  voiceToggleRef.current = () => { if (asrRef.current) void stopVoice(); else void startVoice(); };
  useEffect(() => {
    if (!isNew) return;
    const off = registerVoiceSurface(() => voiceToggleRef.current());
    return () => { off(); void asrRef.current?.stop(); asrRef.current = null; };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [isNew]);

  // 引用项目文件选择器：打开时拉取项目文件清单。
  const openFilePicker = async () => {
    setFilePickerOpen(o => !o);
    if (projectFiles.length === 0) {
      try { setProjectFiles(await listProjectFiles(project.id)); } catch { /* 忽略 */ }
    }
  };
  useEffect(() => {
    if (!filePickerOpen) return;
    const h = (e: PointerEvent) => { if (e.target instanceof Node && filePickerRef.current?.contains(e.target)) return; setFilePickerOpen(false); };
    document.addEventListener('pointerdown', h);
    return () => document.removeEventListener('pointerdown', h);
  }, [filePickerOpen]);

  const doStart = async () => {
    if (!briefInput.trim()) { setErr('请先粘贴大需求内容'); return; }
    if (asrRef.current) await stopVoice();
    setBusy(true); setErr(''); setApplyMsg('');
    // 文本类附件：内容内联进大需求（图片等非文本仅附文件名提示）。
    let composed = briefInput.trim();
    for (const f of attachFiles) {
      if (/^(text\/|application\/(json|xml|x-yaml|yaml)|$)/.test(f.type) || /\.(md|txt|json|ya?ml|csv|log|ts|tsx|js|jsx|rs|py|go|java|toml|html?|css)$/i.test(f.name)) {
        try { const text = await f.text(); composed += `\n\n【附件：${f.name}】\n${text.slice(0, 8000)}`; }
        catch { composed += `\n\n【附件：${f.name}（读取失败）】`; }
      } else {
        composed += `\n\n【附件：${f.name}（非文本，未内联）】`;
      }
    }
    const p = startBlueprintDraft(project.id, composed, refFiles, backend);
    trackInflight(flowKey, p);
    try {
      const v = await p;
      setView(v); setBriefInput(''); setRefFiles([]); setAttachFiles([]); setTab('prd');
      onChanged(v.draft.id);   // 让页面切到这条新需求
    } catch (e) { setErr(String(e)); }
    finally { setBusy(false); }
  };

  const doRefine = async () => {
    if (!draft || !chatInput.trim() || busy) return;
    const ins = chatInput.trim();
    setChatInput(''); setBusy(true); setErr('');
    const prev = view;
    const p = refineBlueprintDraft(draft.id, ins, backend);
    trackInflight(draft.id, p);
    try { const v = await p; flash(diffIds(prev, v)); setView(v); onChanged(); }
    catch (e) { setErr(String(e)); setChatInput(ins); }
    finally { setBusy(false); }
  };

  // P2 断点续跑：起草 Agent 追问挂起时（status=awaiting_answer），回答后清挂起、续跑下一轮。
  const awaiting = draft?.status === 'awaiting_answer';
  const pendingQuestion = awaiting
    ? [...messages].reverse().find(m => m.role === 'question')?.content ?? '起草 Agent 需要你补充一些信息。'
    : '';
  const doAnswer = async () => {
    if (!draft || !chatInput.trim() || busy) return;
    const ans = chatInput.trim();
    setChatInput(''); setBusy(true); setErr('');
    try { const v = await answerBlueprintQuestion(draft.id, ans); setView(v); onChanged(); }
    catch (e) { setErr(String(e)); setChatInput(ans); }
    finally { setBusy(false); }
  };

  // 人工手改：本地乐观更新 + 落库（整份回写）。
  const persist = async (prd: string, specs: BlueprintSpec[], tasklist: BlueprintTask[]) => {
    if (!draft) return;
    setView(v => v ? { ...v, draft: { ...v.draft, prd_markdown: prd, specs, tasklist } } : v);
    try { await patchBlueprintDraft(draft.id, prd, specs, tasklist); onChanged(); }
    catch (e) { setErr(String(e)); }
  };

  const updateSpec = (id: string, patch: Partial<BlueprintSpec>) => {
    if (!draft) return;
    persist(draft.prd_markdown, draft.specs.map(s => s.id === id ? { ...s, ...patch } : s), draft.tasklist);
  };
  const deleteSpec = (id: string) => {
    if (!draft) return;
    persist(draft.prd_markdown, draft.specs.filter(s => s.id !== id), draft.tasklist);
  };
  const addSpec = () => {
    if (!draft) return;
    const s: BlueprintSpec = { id: uid(), category: 'architecture', title: '新规格', content_markdown: '' };
    persist(draft.prd_markdown, [...draft.specs, s], draft.tasklist);
  };
  const updateTask = (id: string, patch: Partial<BlueprintTask>) => {
    if (!draft) return;
    persist(draft.prd_markdown, draft.specs, draft.tasklist.map(t => t.id === id ? { ...t, ...patch } : t));
  };
  const deleteTask = (id: string) => {
    if (!draft) return;
    persist(draft.prd_markdown, draft.specs, draft.tasklist.filter(t => t.id !== id));
  };
  const addTask = () => {
    if (!draft) return;
    const t: BlueprintTask = { id: uid(), title: '新任务', description: '', category: 'Feature', severity: 'medium' };
    persist(draft.prd_markdown, draft.specs, [...draft.tasklist, t]);
  };

  const savePrd = () => { if (draft) { persist(prdDraft, draft.specs, draft.tasklist); } setEditPrd(false); };

  const doApply = async () => {
    if (!draft) return;
    setBusy(true); setErr(''); setApplyMsg('');
    try { const msg = await applyBlueprintDraft(draft.id, writeTasklistDoc); setApplyMsg(msg); }
    catch (e) { setErr(String(e)); }
    finally { setBusy(false); }
  };

  // 编码开发：直接落 issue + CR + 编码执行（仅代码审核）。
  const doCode = async () => {
    if (!draft) return;
    setBusy(true); setErr(''); setApplyMsg('');
    try {
      const crId = await codeBlueprintDraft(draft.id);
      setView(v => v ? { ...v, draft: { ...v.draft, status: 'coding', cr_id: crId } } : v);
      setApplyMsg('已进入编码开发，进度与代码审核请见「变更审核」。');
      onChanged();
    } catch (e) { setErr(String(e)); }
    finally { setBusy(false); }
  };

  const hasDraft = !!draft;
  const phases = hasDraft ? REFINE_PHASES : GEN_PHASES;
  const phaseIdx = Math.min(genPhase, phases.length - 1);

  return (
    <div style={{
      flex: 1, minWidth: 0, background: 'var(--bg)', display: 'flex', flexDirection: 'column', overflow: 'hidden',
    }}>
      {/* 单一头栏：返回 + 需求标题 + 状态 + 操作（含编码开发）*/}
      <div style={{
        display: 'flex', alignItems: 'center', gap: 10, padding: '11px 18px',
        borderBottom: '1px solid var(--border)', flexShrink: 0, background: 'var(--bg-1)',
      }}>
        <button className="icon-btn" onClick={onBack} title="返回需求列表"><Icon name="chevRight" size={16} style={{ transform: 'rotate(180deg)' }} /></button>
        <div style={{ width: 30, height: 30, borderRadius: 8, background: 'var(--ember-tint)', display: 'grid', placeItems: 'center', flexShrink: 0 }}>
          <Icon name="layers" size={15} style={{ color: 'var(--ember)' }} />
        </div>
        <div style={{ minWidth: 0 }}>
          <div style={{ fontFamily: 'var(--font-display)', fontWeight: 700, fontSize: 'var(--text-section)', whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis', maxWidth: 360 }}>
            {isNew ? '新需求改动' : (draft?.title || project.name)}
          </div>
          <div style={{ fontSize: 'var(--text-caption)', color: 'var(--text-3)', fontFamily: 'var(--font-mono)', letterSpacing: '.04em' }}>
            {project.name} · 孵化台
          </div>
        </div>
        {coding && (
          <span className={'chip ' + (DISPLAY_STATUS.coding.chip)} style={{ fontSize: 'var(--text-micro)' }}>{DISPLAY_STATUS.coding.label}</span>
        )}
        <div style={{ flex: 1 }} />
        {hasDraft && !coding && (
          <>
            <button className="btn btn-sm btn-ghost" disabled={busy || noRepo} onClick={doApply}
              title={noRepo ? '项目未配置本地仓库路径，无法写入' : '把 PRD / 规格留档到 .autoforge 工作区'}>
              <Icon name="check" size={13} />写入工作区
            </button>
            <button className="btn btn-primary btn-sm" disabled={busy} onClick={doCode}
              title="直接把当前需求落为编码工单并开始实现（仅代码审核）">
              <Icon name="zap" size={14} />{busy ? '处理中…' : '编码开发'}
            </button>
          </>
        )}
        {hasDraft && coding && draft?.issue_id && onOpenAudit && (
          <button className="btn btn-primary btn-sm" onClick={() => onOpenAudit(project.id, draft.issue_id)}>
            <Icon name="audit" size={14} />去代码审核
          </button>
        )}
        {/* P4 深链：去交付页设计阶段做原型（携 draftId 预选本稿 PRD）。非 primary，尊重每屏≤1 主操作。 */}
        {hasDraft && draft && onOpenDelivery && (
          <button className="btn btn-sm" onClick={() => onOpenDelivery(project.id, { stage: 'design', draftId: draft.id })}
            title="带本稿 PRD 去交付页生成原型设计提示词">
            <Icon name="palette" size={14} />去原型设计 ↗
          </button>
        )}
      </div>

      {applyMsg && (
        <div className="chip green" style={{ display: 'block', margin: '10px 18px 0', padding: '8px 12px', lineHeight: 'var(--leading-normal)' }}>
          ✓ {applyMsg}
        </div>
      )}
      {err && (
        <div style={{ margin: '10px 18px 0', padding: '8px 12px', borderRadius: 9, background: 'var(--red-tint, rgba(220,80,70,.12))', color: 'var(--red)', fontSize: 'var(--text-label)' }}>
          {err}
        </div>
      )}

      {loading ? (
        <div className="empty" style={{ flex: 1 }}><Icon name="refresh" size={28} style={{ opacity: .4 }} /><div>载入草稿…</div></div>
      ) : busy && !hasDraft ? (
        // ── 起草进行中（含切页回来重连）：左进度步骤 / 右骨架预览，预演最终布局，不丢状态 ──
        <div style={{ flex: 1, display: 'flex', minHeight: 0 }}>
          {/* 左：阶段进度 */}
          <div style={{ width: '40%', minWidth: 320, maxWidth: 520, borderRight: '1px solid var(--border)', background: 'var(--bg-1)', padding: '22px 20px', display: 'flex', flexDirection: 'column', gap: 18 }}>
            <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
              <Icon name="zap" size={16} style={{ color: 'var(--ember)' }} />
              <span style={{ fontFamily: 'var(--font-display)', fontWeight: 700, fontSize: 'var(--text-title)' }}>正在起草《{project.name}》</span>
            </div>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
              {GEN_PHASES.map((label, i) => {
                const done = i < phaseIdx, current = i === phaseIdx;
                return (
                  <div key={i} style={{ display: 'flex', alignItems: 'center', gap: 10, opacity: i > phaseIdx ? .4 : 1, transition: 'opacity .3s' }}>
                    <span style={{ width: 18, height: 18, display: 'grid', placeItems: 'center', flexShrink: 0 }}>
                      {done
                        ? <Icon name="check" size={13} style={{ color: 'var(--green)' }} />
                        : current
                          ? <Icon name="refresh" size={13} style={{ color: 'var(--ember)', animation: 'spin 1s linear infinite' }} />
                          : <span style={{ width: 6, height: 6, borderRadius: '50%', background: 'var(--text-faint)' }} />}
                    </span>
                    <span style={{ fontSize: 'var(--text-label)', color: current ? 'var(--text)' : 'var(--text-2)', fontWeight: current ? 600 : 400 }}>{label}</span>
                  </div>
                );
              })}
            </div>
            <div style={{ marginTop: 'auto', display: 'flex', alignItems: 'center', gap: 7, fontSize: 'var(--text-micro)', color: 'var(--text-faint)', fontFamily: 'var(--font-mono)' }}>
              <span className="typing" aria-hidden><i /><i /><i /></span>切到其他页面也不会中断，回来即恢复
            </div>
          </div>
          {/* 右：骨架预览，预示 PRD/规格/任务最终布局 */}
          <div style={{ flex: 1, padding: '16px 20px', display: 'flex', flexDirection: 'column', gap: 14, minWidth: 0, overflow: 'hidden' }}>
            <div style={{ display: 'flex', gap: 8 }}>
              {[60, 64, 60].map((w, i) => <div key={i} className="skel" style={{ height: 30, width: w, borderRadius: 8 }} />)}
            </div>
            <div className="skel" style={{ height: 24, width: '46%', marginTop: 4 }} />
            <div className="skel" style={{ height: 13, width: '94%' }} />
            <div className="skel" style={{ height: 13, width: '88%' }} />
            <div className="skel" style={{ height: 13, width: '72%' }} />
            {[0, 1, 2].map(i => (
              <div key={i} className="panel" style={{ padding: '12px 13px', display: 'flex', flexDirection: 'column', gap: 8 }}>
                <div style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
                  <div className="skel" style={{ height: 18, width: 72, borderRadius: 99 }} />
                  <div className="skel" style={{ height: 14, width: '38%' }} />
                </div>
                <div className="skel" style={{ height: 12, width: '92%' }} />
                <div className="skel" style={{ height: 12, width: '78%' }} />
              </div>
            ))}
          </div>
        </div>
      ) : !hasDraft ? (
        // ── 起草态：左 composer / 右引导，铺满整宽 ──
        <div style={{ flex: 1, overflowY: 'auto', display: 'flex', justifyContent: 'center' }}>
          <div className="rise" style={{ width: 'min(720px, 92%)', margin: '12vh auto 40px', display: 'flex', flexDirection: 'column', gap: 16, alignSelf: 'flex-start' }}>
            <div style={{ textAlign: 'center' }}>
              <div style={{ fontFamily: 'var(--font-display)', fontWeight: 800, fontSize: 'var(--text-page-title)', letterSpacing: '-.01em' }}>
                <Icon name="layers" size={24} style={{ color: 'var(--ember)', verticalAlign: '-3px', marginRight: 8 }} />新建需求改动
              </div>
              <div style={{ marginTop: 8, fontSize: 'var(--text-body)', color: 'var(--text-3)' }}>
                把一段大需求改动写清楚 —— AI 会炼成 PRD + 技术规格 + 任务清单。
              </div>
            </div>
            {noRepo && (
              <div className="chip amber" style={{ display: 'block', padding: '8px 12px', lineHeight: 'var(--leading-normal)' }}>
                该项目未配置本地仓库路径，可生成蓝图，但「写入工作区/引用文件」会受限。
              </div>
            )}

            {/* 统一 composer 盒：输入 + 引用/附件/语音 工具条 */}
            <div style={{
              background: 'var(--bg-2)', border: `1px solid ${inputFocused ? 'var(--ember)' : 'var(--border-strong)'}`,
              borderRadius: 'var(--radius-lg, 16px)', boxShadow: inputFocused ? '0 0 0 3px var(--ember-tint)' : 'var(--shadow)',
              transition: 'border-color .14s, box-shadow .14s', padding: '12px 14px',
            }}>
              {/* 已引用文件 / 附件 chips */}
              {(refFiles.length > 0 || attachFiles.length > 0) && (
                <div style={{ display: 'flex', flexWrap: 'wrap', gap: 6, marginBottom: 10 }}>
                  {refFiles.map(f => (
                    <span key={'r' + f} className="chip" style={{ background: 'var(--ember-tint)', color: 'var(--ember)', borderColor: 'transparent', maxWidth: 220 }}>
                      <Icon name="file" size={11} /><span style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{f.split('/').pop()}</span>
                      <span style={{ cursor: 'pointer', display: 'inline-flex', flexShrink: 0 }} onClick={() => setRefFiles(p => p.filter(x => x !== f))}><Icon name="x" size={11} /></span>
                    </span>
                  ))}
                  {attachFiles.map((f, i) => (
                    <span key={'a' + i} className="chip" style={{ background: 'var(--bg-3)', color: 'var(--text-2)', borderColor: 'var(--border-strong)', maxWidth: 220 }}>
                      <Icon name="image" size={11} /><span style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{f.name}</span>
                      <span style={{ cursor: 'pointer', display: 'inline-flex', flexShrink: 0 }} onClick={() => setAttachFiles(p => p.filter((_, j) => j !== i))}><Icon name="x" size={11} /></span>
                    </span>
                  ))}
                </div>
              )}
              <textarea
                rows={5} value={briefInput} onChange={e => setBriefInput(e.target.value)} autoFocus
                onFocus={() => setInputFocused(true)} onBlur={() => setInputFocused(false)}
                onKeyDown={e => { if (e.key === 'Enter' && (e.metaKey || e.ctrlKey) && briefInput.trim()) { e.preventDefault(); void doStart(); } }}
                placeholder="描述这次大需求改动 —— 背景、目标、要支持的能力、约束…越具体越好。"
                style={{ width: '100%', resize: 'vertical', minHeight: 120, maxHeight: 360, border: 'none', background: 'transparent', outline: 'none', padding: 0, fontFamily: 'var(--font-sans)', fontSize: 'var(--text-body)', lineHeight: 'var(--leading-relaxed)', color: 'var(--text)' }}
              />
              {/* 工具条 */}
              <div style={{ display: 'flex', alignItems: 'center', gap: 6, marginTop: 10 }}>
                {/* 引用项目文件 */}
                <div style={{ position: 'relative' }} ref={filePickerRef}>
                  <button className="icon-btn" onClick={openFilePicker} title="引用项目文件作为上下文">
                    <Icon name="plus" size={17} />
                  </button>
                  {filePickerOpen && (
                    <div className="mention-pop" style={{ left: 0, right: 'auto', bottom: 'calc(100% + 6px)', top: 'auto', width: 320, maxHeight: 360, overflowY: 'auto', marginBottom: 0, zIndex: 80 }}>
                      <div className="mention-pop-label" style={{ display: 'block', padding: '6px 10px' }}>引用项目文件（注入起草上下文）</div>
                      <div style={{ padding: '4px 8px' }}>
                        <input value={fileFilter} onChange={e => setFileFilter(e.target.value)} placeholder="🔍 过滤文件…" autoFocus
                          style={{ width: '100%', fontSize: 'var(--text-label)' }} />
                      </div>
                      {projectFiles.filter(f => !fileFilter || f.rel_path.toLowerCase().includes(fileFilter.toLowerCase())).slice(0, 200).map(f => {
                        const on = refFiles.includes(f.rel_path);
                        return (
                          <div key={f.rel_path} className={'mention-row' + (on ? ' mention-active' : '')}
                            onClick={() => setRefFiles(p => on ? p.filter(x => x !== f.rel_path) : [...p, f.rel_path])}>
                            <Icon name={on ? 'check' : 'file'} size={13} style={{ color: on ? 'var(--ember)' : 'var(--text-3)', flexShrink: 0 }} />
                            <div style={{ minWidth: 0, flex: 1 }}><div className="nm" style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{f.rel_path}</div></div>
                          </div>
                        );
                      })}
                      {projectFiles.length === 0 && <div style={{ padding: '10px 12px', fontSize: 'var(--text-label)', color: 'var(--text-faint)' }}>无可引用文件（项目未配置仓库或为空）</div>}
                    </div>
                  )}
                </div>
                {/* 本地附件 */}
                <input ref={attachInputRef} type="file" multiple style={{ display: 'none' }}
                  onChange={e => { const fs = Array.from(e.target.files ?? []); if (fs.length) setAttachFiles(p => [...p, ...fs]); if (attachInputRef.current) attachInputRef.current.value = ''; }} />
                <button className="icon-btn" onClick={() => attachInputRef.current?.click()} title="附加本地文件（文本类内容会内联）">
                  <Icon name="image" size={16} />
                </button>
                {/* 语音录入 */}
                <button className={'icon-btn' + (voiceOn ? ' on' : '')} onClick={() => (voiceOn ? void stopVoice() : void startVoice())}
                  title={voiceOn ? '停止语音录入' : '语音录入（实时转写，快捷键同全局语音）'} style={voiceOn ? { color: 'var(--ember)' } : undefined}>
                  <Icon name="mic" size={16} />
                </button>
                {(voiceOn || voiceConnecting) && (
                  <span style={{ fontSize: 'var(--text-micro)', color: 'var(--ember)', fontFamily: 'var(--font-mono)' }}>{voiceConnecting ? '连接中…' : '录音中'}</span>
                )}
                <div style={{ flex: 1 }} />
                {/* 分析后端：需求分析专家(LLM) / 编码 Agent(读真实仓库)。无仓库时仅 LLM。 */}
                <div className="seg" title="选择由谁来分析并起草这条需求">
                  <button className={backend === 'analysis' ? 'on' : ''} onClick={() => setBackend('analysis')}>需求分析专家</button>
                  <button className={backend === 'code_agent' ? 'on' : ''} disabled={!project.repo_path?.trim()}
                    title={!project.repo_path?.trim() ? '该项目未配置仓库路径，无法用编码 Agent 读代码起草' : '用项目配置的编码 Agent 只读跑真实仓库起草（更贴实际代码）'}
                    onClick={() => setBackend('code_agent')}>编码 Agent</button>
                </div>
                <span style={{ fontSize: 'var(--text-micro)', color: 'var(--text-faint)', fontFamily: 'var(--font-mono)' }}>⌘/Ctrl+Enter</span>
                <button className="btn btn-primary btn-sm" onClick={doStart} disabled={busy || !briefInput.trim()}>
                  <Icon name="zap" size={14} />{busy ? '起草中…' : 'AI 起草'}
                </button>
              </div>
            </div>

            {/* 快速示例 */}
            <div style={{ display: 'flex', alignItems: 'center', gap: 7, flexWrap: 'wrap', justifyContent: 'center' }}>
              {EXAMPLE_BRIEFS.map(ex => (
                <button key={ex.label} className="chip" title={ex.text} onClick={() => setBriefInput(ex.text)}
                  style={{ cursor: 'pointer', appearance: 'none', WebkitAppearance: 'none', background: 'var(--bg-3)', color: 'var(--text-2)', borderColor: 'var(--border-strong)' }}>
                  <Icon name="plus" size={11} style={{ color: 'var(--ember)' }} />{ex.label}
                </button>
              ))}
            </div>
          </div>
        </div>
      ) : (
        // ── 工作台态：左对话 / 右预览 ──
        <div style={{ flex: 1, display: 'flex', minHeight: 0 }}>
          {/* 左：对话流 */}
          <div style={{ width: '40%', minWidth: 320, maxWidth: 520, display: 'flex', flexDirection: 'column', borderRight: '1px solid var(--border)', background: 'var(--bg-1)' }}>
            <div style={{ flex: 1, overflowY: 'auto', padding: '14px 16px', display: 'flex', flexDirection: 'column', gap: 12 }}>
              {messages.map(m => {
                // 人侧（user/answer）靠右熔岩气泡；机侧（assistant/question/eval）靠左。
                const mine = m.role === 'user' || m.role === 'answer';
                // 各角色标签 + 语义色（question=amber 追问 / eval=green 评估 / assistant=ember 蓝图 Agent）。
                const meta =
                  m.role === 'assistant' ? { label: '蓝图 Agent', color: 'var(--ember)' }
                  : m.role === 'question' ? { label: '追问', color: 'var(--amber)' }
                  : m.role === 'eval' ? { label: '评估', color: 'var(--green)' }
                  : null;
                return (
                  <div key={m.id} style={{ display: 'flex', justifyContent: mine ? 'flex-end' : 'flex-start' }}>
                    <div style={{
                      maxWidth: '88%', padding: '9px 12px', borderRadius: 12, fontSize: 'var(--text-label)',
                      lineHeight: 'var(--leading-relaxed)', whiteSpace: 'pre-wrap', wordBreak: 'break-word',
                      background: mine ? 'var(--bubble-me)' : 'var(--bubble-them)',
                      color: mine ? 'var(--bubble-me-text)' : 'var(--bubble-them-text)',
                      border: mine ? 'none' : '1px solid var(--border)',
                    }}>
                      {meta && (
                        <div style={{ display: 'flex', alignItems: 'center', gap: 5, marginBottom: 4, color: meta.color, fontFamily: 'var(--font-mono)', fontSize: 'var(--text-micro)', textTransform: 'uppercase', letterSpacing: '.1em' }}>
                          <Icon name="zap" size={11} />{meta.label}
                        </div>
                      )}
                      {m.content}
                    </div>
                  </div>
                );
              })}
              {busy && (
                <div style={{ display: 'flex', justifyContent: 'flex-start' }}>
                  <div style={{ display: 'flex', alignItems: 'center', gap: 8, padding: '9px 12px', borderRadius: 12, background: 'var(--bg-3)', border: '1px solid var(--border)', color: 'var(--text-2)', fontSize: 'var(--text-label)' }}>
                    <span className="typing" aria-hidden><i /><i /><i /></span>
                    <span key={phaseIdx} className="cs-phase" style={{ fontFamily: 'var(--font-mono)', letterSpacing: '.02em' }}>{phases[phaseIdx]}</span>
                  </div>
                </div>
              )}
              <div ref={chatEndRef} />
            </div>
            {/* 输入框：统一 composer（输入 + 发送同框，focus 走 ember 光环）*/}
            <div style={{ borderTop: '1px solid var(--border)', padding: '12px', flexShrink: 0, background: 'var(--bg-1)' }}>
              {/* P2 追问卡：起草 Agent 挂起等答复（amber 语义色，非装饰）*/}
              {awaiting && (
                <div style={{ marginBottom: 10, padding: '10px 12px', borderRadius: 12, background: 'var(--amber-tint)', border: '1px solid var(--amber-soft)' }}>
                  <div style={{ marginBottom: 6, color: 'var(--amber)', fontFamily: 'var(--font-mono)', fontSize: 'var(--text-micro)', textTransform: 'uppercase', letterSpacing: '.1em' }}>
                    起草 Agent 追问 · 待答复
                  </div>
                  <div style={{ fontSize: 'var(--text-label)', color: 'var(--text)', lineHeight: 'var(--leading-relaxed)', whiteSpace: 'pre-wrap' }}>{pendingQuestion}</div>
                </div>
              )}
              <div style={{
                display: 'flex', alignItems: 'flex-end', gap: 8,
                background: 'var(--bg-3)', borderRadius: 12, padding: '6px 6px 6px 12px',
                border: `1px solid ${inputFocused && !coding ? 'var(--ember)' : 'var(--border-strong)'}`,
                boxShadow: inputFocused && !coding ? '0 0 0 3px var(--ember-tint)' : 'none',
                transition: 'border-color .14s, box-shadow .14s', opacity: coding ? .6 : 1,
              }}>
                <textarea
                  rows={2} value={chatInput} onChange={e => setChatInput(e.target.value)}
                  onFocus={() => setInputFocused(true)} onBlur={() => setInputFocused(false)}
                  onKeyDown={e => { if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) { e.preventDefault(); (awaiting ? doAnswer : doRefine)(); } }}
                  disabled={busy || coding}
                  placeholder={coding ? '已进入编码开发，如需调整请到「变更审核」或新建一条需求改动' : awaiting ? '回答上面的追问，Agent 将据此继续起草…' : '告诉我要改哪里：「验收标准写细点」「支付拆成 3 个任务」「补一条限流规格」…'}
                  style={{
                    flex: 1, resize: 'none', minHeight: 40, maxHeight: 160,
                    border: 'none', background: 'transparent', outline: 'none', padding: '6px 0',
                    fontFamily: 'var(--font-sans)', fontSize: 'var(--text-label)', lineHeight: 'var(--leading-normal)', color: 'var(--text)',
                  }}
                />
                <button className="btn btn-primary" style={{ width: 36, height: 36, padding: 0, borderRadius: 9, flexShrink: 0, justifyContent: 'center' }}
                  onClick={awaiting ? doAnswer : doRefine} disabled={busy || coding || !chatInput.trim()} title={awaiting ? '提交答复（⌘/Ctrl+Enter）' : '发送修正指令（⌘/Ctrl+Enter）'}>
                  <Icon name="send" size={16} />
                </button>
              </div>
              {!coding && (
                <div style={{ marginTop: 6, paddingLeft: 2, fontSize: 'var(--text-micro)', color: 'var(--text-faint)', fontFamily: 'var(--font-mono)', letterSpacing: '.03em' }}>
                  ⌘ / Ctrl + Enter 发送
                </div>
              )}
            </div>
          </div>

          {/* 右：产物预览 */}
          <div style={{ flex: 1, display: 'flex', flexDirection: 'column', minWidth: 0 }}>
            <div style={{ display: 'flex', alignItems: 'center', gap: 10, padding: '12px 18px 0', flexShrink: 0 }}>
              <div className="seg">
                {([['prd', 'PRD'], ['specs', `规格 · ${draft!.specs.length}`], ['tasks', `任务 · ${draft!.tasklist.length}`]] as const).map(([id, label]) => (
                  <button key={id} className={tab === id ? 'on' : ''} onClick={() => setTab(id as typeof tab)}>{label}</button>
                ))}
              </div>
            </div>
            <div style={{ flex: 1, overflowY: 'auto', padding: '14px 18px 24px' }}>
              {tab === 'prd' && (
                <PrdView
                  prd={draft!.prd_markdown} editing={editPrd} prdDraft={prdDraft} flashed={changed.has(PRD_KEY)} disabled={coding}
                  onEdit={() => { setPrdDraft(draft!.prd_markdown); setEditPrd(true); }}
                  onChange={setPrdDraft} onSave={savePrd} onCancel={() => setEditPrd(false)}
                />
              )}
              {tab === 'specs' && (
                <SpecsView
                  specs={draft!.specs} changed={changed} disabled={coding}
                  onUpdate={updateSpec} onDelete={deleteSpec} onAdd={addSpec}
                />
              )}
              {tab === 'tasks' && (
                <TasksView
                  tasks={draft!.tasklist} changed={changed} disabled={coding}
                  writeTasklistDoc={writeTasklistDoc}
                  onUpdate={updateTask} onDelete={deleteTask} onAdd={addTask}
                  onToggleDoc={setWriteTasklistDoc}
                />
              )}
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

// ── PRD 视图 ──
function PrdView({ prd, editing, prdDraft, flashed, disabled, onEdit, onChange, onSave, onCancel }: {
  prd: string; editing: boolean; prdDraft: string; flashed: boolean; disabled: boolean;
  onEdit: () => void; onChange: (v: string) => void; onSave: () => void; onCancel: () => void;
}) {
  return (
    <div>
      <div style={{ display: 'flex', alignItems: 'center', marginBottom: 10 }}>
        <span style={{ fontFamily: 'var(--font-mono)', fontSize: 'var(--text-caption)', letterSpacing: '.14em', textTransform: 'uppercase', color: 'var(--text-3)' }}>产品需求文档 · docs/PRD.md</span>
        <div style={{ flex: 1 }} />
        {!disabled && !editing && (
          <button className="btn btn-ghost btn-sm" onClick={onEdit}><Icon name="edit" size={13} />编辑</button>
        )}
        {editing && (
          <div style={{ display: 'flex', gap: 6 }}>
            <button className="btn btn-ghost btn-sm" onClick={onCancel}>取消</button>
            <button className="btn btn-sm" onClick={onSave}><Icon name="check" size={13} />保存</button>
          </div>
        )}
      </div>
      {editing ? (
        <textarea value={prdDraft} onChange={e => onChange(e.target.value)} autoFocus
          style={{ width: '100%', minHeight: 'calc(100vh - 280px)', resize: 'vertical', fontFamily: 'var(--font-mono)', fontSize: 'var(--text-label)', lineHeight: 'var(--leading-relaxed)' }} />
      ) : prd.trim() ? (
        <div className="bubble doc" style={{
          maxWidth: 'var(--measure, 780px)', padding: '4px 2px', background: 'transparent', border: 'none',
          ...(flashed ? { outline: '2px solid var(--ember)', outlineOffset: 6, borderRadius: 10 } : {}),
        }}>
          <Markdown md={prd} />
        </div>
      ) : (
        <div className="empty"><Icon name="inbox" size={28} style={{ opacity: .3 }} /><div>AI 未生成 PRD</div></div>
      )}
    </div>
  );
}

// ── 规格视图 ──
function SpecsView({ specs, changed, disabled, onUpdate, onDelete, onAdd }: {
  specs: BlueprintSpec[]; changed: Set<string>; disabled: boolean;
  onUpdate: (id: string, p: Partial<BlueprintSpec>) => void; onDelete: (id: string) => void; onAdd: () => void;
}) {
  const [editId, setEditId] = useState<string | null>(null);
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 10, maxWidth: 'var(--measure, 780px)' }}>
      <div style={{ fontFamily: 'var(--font-mono)', fontSize: 'var(--text-caption)', letterSpacing: '.14em', textTransform: 'uppercase', color: 'var(--text-3)' }}>技术规格 · 登记到项目规格 + .autoforge/specs/</div>
      {specs.length === 0 && <div className="empty" style={{ padding: '24px 0' }}><Icon name="layers" size={26} style={{ opacity: .3 }} /><div>暂无规格</div></div>}
      {specs.map(s => {
        const editing = editId === s.id;
        return (
          <div key={s.id} className={'panel' + (changed.has(s.id) ? ' rise' : '')} style={{
            padding: '11px 13px', ...(changed.has(s.id) ? { borderLeft: '2px solid var(--ember)' } : {}),
          }}>
            <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: editing ? 8 : 5 }}>
              <MiniSelect value={s.category} options={SPEC_CATEGORIES} disabled={disabled} onChange={v => onUpdate(s.id, { category: v })} />
              {editing ? (
                <input value={s.title} onChange={e => onUpdate(s.id, { title: e.target.value })}
                  style={{ flex: 1, fontSize: 'var(--text-control)', fontWeight: 600 }} />
              ) : (
                <span style={{ flex: 1, fontSize: 'var(--text-control)', fontWeight: 600 }}>{s.title}</span>
              )}
              {!disabled && (
                <>
                  <button className="icon-btn" style={{ width: 26, height: 26 }} onClick={() => setEditId(editing ? null : s.id)} title={editing ? '完成编辑' : '编辑'}>
                    <Icon name={editing ? 'check' : 'edit'} size={12} />
                  </button>
                  <button className="icon-btn" style={{ width: 26, height: 26, color: 'var(--red)' }} onClick={() => onDelete(s.id)} title="删除"><Icon name="trash" size={12} /></button>
                </>
              )}
            </div>
            {editing ? (
              <textarea value={s.content_markdown} onChange={e => onUpdate(s.id, { content_markdown: e.target.value })}
                style={{ width: '100%', minHeight: 90, resize: 'vertical', fontFamily: 'var(--font-mono)', fontSize: 'var(--text-label)' }} />
            ) : (
              <div style={{ fontSize: 'var(--text-label)', color: 'var(--text-2)' }}><Markdown md={s.content_markdown || '（空）'} /></div>
            )}
          </div>
        );
      })}
      {!disabled && (
        <button className="btn btn-ghost btn-sm" style={{ alignSelf: 'flex-start' }} onClick={onAdd}><Icon name="plus" size={13} />新增规格</button>
      )}
    </div>
  );
}

// ── 任务视图 ──
function TasksView({ tasks, changed, disabled, writeTasklistDoc, onUpdate, onDelete, onAdd, onToggleDoc }: {
  tasks: BlueprintTask[]; changed: Set<string>; disabled: boolean;
  writeTasklistDoc: boolean;
  onUpdate: (id: string, p: Partial<BlueprintTask>) => void; onDelete: (id: string) => void; onAdd: () => void;
  onToggleDoc: (v: boolean) => void;
}) {
  const [editId, setEditId] = useState<string | null>(null);
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 7, maxWidth: 'var(--measure, 780px)' }}>
      <div style={{ display: 'flex', alignItems: 'center' }}>
        <span style={{ fontFamily: 'var(--font-mono)', fontSize: 'var(--text-caption)', letterSpacing: '.14em', textTransform: 'uppercase', color: 'var(--text-3)' }}>任务清单 · 编码开发时整条需求一起实现</span>
        <div style={{ flex: 1 }} />
        <span style={{ fontFamily: 'var(--font-mono)', fontSize: 'var(--text-micro)', color: 'var(--text-faint)' }}>共 {tasks.length}</span>
      </div>
      {tasks.length === 0 && <div className="empty" style={{ padding: '24px 0' }}><Icon name="list" size={26} style={{ opacity: .3 }} /><div>暂无任务</div></div>}
      {tasks.map(t => {
        const editing = editId === t.id;
        return (
          <div key={t.id} className={'panel' + (changed.has(t.id) ? ' rise' : '')} style={{
            display: 'flex', alignItems: 'flex-start', gap: 10, padding: '9px 12px',
            ...(changed.has(t.id) ? { borderLeft: '2px solid var(--ember)' } : {}),
          }}>
            <Icon name="list" size={14} style={{ color: 'var(--text-faint)', marginTop: 4, flexShrink: 0 }} />
            <div style={{ flex: 1, minWidth: 0 }}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 6, flexWrap: 'wrap' }}>
                {editing ? (
                  <input value={t.title} onChange={e => onUpdate(t.id, { title: e.target.value })} style={{ flex: 1, minWidth: 160, fontSize: 'var(--text-control)', fontWeight: 600 }} />
                ) : (
                  <span style={{ fontSize: 'var(--text-control)', fontWeight: 600 }}>{t.title}</span>
                )}
                <MiniSelect value={t.category} options={TASK_CATEGORIES} disabled={disabled} onChange={v => onUpdate(t.id, { category: v })} />
                <MiniSelect value={t.severity} options={SEVERITIES} disabled={disabled} onChange={v => onUpdate(t.id, { severity: v })} />
                {!disabled && (
                  <>
                    <button className="icon-btn" style={{ width: 24, height: 24 }} onClick={() => setEditId(editing ? null : t.id)} title={editing ? '完成' : '编辑'}>
                      <Icon name={editing ? 'check' : 'edit'} size={11} />
                    </button>
                    <button className="icon-btn" style={{ width: 24, height: 24, color: 'var(--red)' }} onClick={() => onDelete(t.id)} title="删除"><Icon name="trash" size={11} /></button>
                  </>
                )}
              </div>
              {editing ? (
                <textarea value={t.description} onChange={e => onUpdate(t.id, { description: e.target.value })} placeholder="一两句交代要做什么"
                  style={{ width: '100%', marginTop: 5, minHeight: 48, resize: 'vertical', fontFamily: 'var(--font-sans)', fontSize: 'var(--text-label)' }} />
              ) : t.description ? (
                <div style={{ fontSize: 'var(--text-label)', color: 'var(--text-3)', marginTop: 2 }}>{t.description}</div>
              ) : null}
            </div>
          </div>
        );
      })}
      {!disabled && (
        <button className="btn btn-ghost btn-sm" style={{ alignSelf: 'flex-start', marginTop: 2 }} onClick={onAdd}><Icon name="plus" size={13} />新增任务</button>
      )}
      <label style={{ display: 'flex', alignItems: 'center', gap: 8, marginTop: 8, fontSize: 'var(--text-label)', color: 'var(--text-2)', cursor: 'pointer' }}>
        <input type="checkbox" checked={writeTasklistDoc} disabled={disabled} onChange={e => onToggleDoc(e.target.checked)} style={{ accentColor: 'var(--ember)' }} />
        写入工作区时同时把任务清单写入 .autoforge/docs/TASKLIST.md
      </label>
    </div>
  );
}

// ── 小型自定义下拉（替代原生 select，复用 mention-pop 模式）──
function MiniSelect({ value, options, disabled, onChange }: {
  value: string; options: string[]; disabled?: boolean; onChange: (v: string) => void;
}) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const h = (e: MouseEvent) => { if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false); };
    document.addEventListener('mousedown', h);
    return () => document.removeEventListener('mousedown', h);
  }, []);
  return (
    <div ref={ref} style={{ position: 'relative' }}>
      <button className="chip" style={{
        cursor: disabled ? 'default' : 'pointer', fontSize: 'var(--text-micro)',
        display: 'inline-flex', alignItems: 'center', gap: 3, opacity: disabled ? .7 : 1,
        // 显式重置原生按钮外观 + 给出表面色：裸 .chip 在 <button> 上会漏出浏览器默认
        // 浅灰底，导致 var(--text-2) 文字看不清；这里强制 bg-3 深色表面 + 高对比文字。
        appearance: 'none', WebkitAppearance: 'none',
        background: 'var(--bg-3)', color: 'var(--text)', borderColor: 'var(--border-strong)',
      }}
        disabled={disabled} onClick={() => setOpen(o => !o)}>
        {value}{!disabled && <Icon name="chevDown" size={10} style={{ color: 'var(--text-3)' }} />}
      </button>
      {open && (
        <div className="mention-pop" style={{ left: 0, right: 'auto', top: 'calc(100% + 4px)', bottom: 'auto', minWidth: 130, marginBottom: 0, zIndex: 60 }}>
          {options.map(o => (
            <div key={o} className={'mention-row' + (o === value ? ' mention-active' : '')} onClick={() => { onChange(o); setOpen(false); }}>{o}</div>
          ))}
        </div>
      )}
    </div>
  );
}
