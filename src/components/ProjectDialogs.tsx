import React, { useState } from 'react';
import Icon from './Icon';
import { cloneProjectFromGit, createLocalProject, updateProject, type Project } from '../services';

const slugify = (name: string) => {
  const slug = name.trim().toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-+|-+$/g, '');
  return slug || `project-${Date.now().toString(36)}`;
};

type ProjectCreateMode = 'local' | 'git';

// 运行配置（dev/prod/test/quality/preview/deploy…）统一在「项目管理 → 运行配置」tab
// 维护，见 src/utils/projectConfig.ts。创建/编辑弹窗只负责项目身份信息。

// ── ProjectCreateModal ────────────────────────────────────────────────────────

export function ProjectCreateModal({ onClose, onCreated }: {
  onClose: () => void;
  onCreated: (project: Project) => void;
}) {
  const [mode, setMode] = useState<ProjectCreateMode>('local');
  const [form, setForm] = useState({
    name: '', slug: '', description: '', repo_path: '',
    git_url: '', target_path: '', clone_branch: '', git_username: '', git_password: '',
    branch_dev: 'dev', branch_main: 'main',
  });
  const [slugTouched, setSlugTouched] = useState(false);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState('');

  const updateName = (name: string) =>
    setForm(f => ({ ...f, name, slug: slugTouched ? f.slug : slugify(name) }));

  const submit = async () => {
    const name = form.name.trim();
    const slug = form.slug.trim();
    const repoPath = form.repo_path.trim();
    const targetPath = form.target_path.trim();
    const gitUrl = form.git_url.trim();
    const gitUsername = form.git_username.trim();
    const gitPassword = form.git_password;
    if (!name) { setError('项目名称不能为空'); return; }
    if (!slug) { setError('项目标识不能为空'); return; }
    if (mode === 'local' && !repoPath) { setError('本地目录不能为空'); return; }
    if (mode === 'git' && !gitUrl) { setError('Git 地址不能为空'); return; }
    if (mode === 'git' && !targetPath) { setError('克隆到本地目录不能为空'); return; }
    if (mode === 'git' && (!!gitUsername !== !!gitPassword)) { setError('Git 认证需要同时填写用户名和密码/Token'); return; }
    setLoading(true); setError('');
    try {
      const common = {
        name, slug,
        description: form.description.trim(),
        branch_dev: form.branch_dev.trim() || 'dev',
        branch_main: form.branch_main.trim() || 'main',
      };
      const project = mode === 'local'
        ? await createLocalProject({ ...common, repo_path: repoPath })
        : await cloneProjectFromGit({
            ...common,
            git_url: gitUrl,
            target_path: targetPath,
            clone_branch: form.clone_branch.trim() || undefined,
            git_username: gitUsername || undefined,
            git_password: gitPassword || undefined,
          });
      onCreated(project);
    } catch (e) { setError(String(e)); }
    finally { setLoading(false); }
  };

  return (
    <div style={{ position: 'fixed', inset: 'var(--win-gutter,0)', borderRadius: 14, background: 'rgba(0,0,0,.5)', backdropFilter: 'blur(3px)', display: 'grid', placeItems: 'center', zIndex: 220 }}>
      <div style={{ width: 560, maxHeight: 'min(760px, calc(100vh - 32px))', background: 'var(--bg-2)', border: '1px solid var(--border-strong)', borderRadius: 18, boxShadow: 'var(--shadow-lg)', overflow: 'hidden', display: 'flex', flexDirection: 'column' }} onClick={e => e.stopPropagation()}>
        <div style={{ padding: '18px 20px 14px', borderBottom: '1px solid var(--border)', display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
          <div className="eyebrow" style={{ fontSize: 'var(--text-section)' }}><span className="cn">添加项目</span></div>
          <button className="icon-btn" onClick={onClose}><Icon name="x" size={18} /></button>
        </div>
        <div className="scroll" style={{ padding: '16px 20px', overflowY: 'auto', minHeight: 0 }}>
          <div className="seg" style={{ marginBottom: 14 }}>
            <button className={mode === 'local' ? 'on' : ''} onClick={() => setMode('local')}>
              <Icon name="folderPlus" size={13} style={{ verticalAlign: -2, marginRight: 4 }} />本地新建
            </button>
            <button className={mode === 'git' ? 'on' : ''} onClick={() => setMode('git')}>
              <Icon name="download" size={13} style={{ verticalAlign: -2, marginRight: 4 }} />从 Git 拉取
            </button>
          </div>
          {mode === 'git' && (
            <div className="cfg-fields" style={{ gridTemplateColumns: '1fr 1fr', gap: 10, marginBottom: 10 }}>
              <div className="field full">
                <label>Git 仓库地址</label>
                <input value={form.git_url} onChange={e => setForm(f => ({ ...f, git_url: e.target.value }))} placeholder="https://github.com/org/repo.git" />
              </div>
              <div className="field full">
                <label>克隆到本地目录</label>
                <input value={form.target_path} onChange={e => setForm(f => ({ ...f, target_path: e.target.value }))} placeholder="/home/user/projects/repo" />
              </div>
              <div className="field full">
                <label>克隆分支（可选）</label>
                <input value={form.clone_branch} onChange={e => setForm(f => ({ ...f, clone_branch: e.target.value }))} placeholder="留空使用远端默认分支" />
              </div>
              <div className="field">
                <label>Git 用户名（私有仓库可选）</label>
                <input value={form.git_username} onChange={e => setForm(f => ({ ...f, git_username: e.target.value }))} placeholder="GitHub 用户名" />
              </div>
              <div className="field">
                <label>密码 / Token（私有仓库可选）</label>
                <input type="password" value={form.git_password} onChange={e => setForm(f => ({ ...f, git_password: e.target.value }))} placeholder="建议使用 Personal Access Token" />
              </div>
            </div>
          )}
          <ProjectFormFields
            form={form}
            setForm={setForm}
            slugTouched={slugTouched}
            setSlugTouched={setSlugTouched}
            updateName={updateName}
            showSlug
            showRepoPath={mode === 'local'}
            repoPathLabel="本地目录"
            repoPathPlaceholder="/home/user/projects/my-app"
          />
          {error && <div style={{ color: 'var(--red)', fontSize: 'var(--text-control)', marginTop: 10 }}>{error}</div>}
        </div>
        <div style={{ padding: '14px 20px', borderTop: '1px solid var(--border)', display: 'flex', gap: 10, justifyContent: 'flex-end' }}>
          <button className="btn" onClick={onClose}>取消</button>
          <button className="btn btn-primary" onClick={submit} disabled={loading}>
            <Icon name={mode === 'git' ? 'download' : 'plus'} size={15} />{loading ? (mode === 'git' ? '拉取中...' : '创建中...') : (mode === 'git' ? '拉取并添加' : '创建项目')}
          </button>
        </div>
      </div>
    </div>
  );
}

// ── ProjectEditModal ──────────────────────────────────────────────────────────

export function ProjectEditModal({ project, onClose, onSaved }: {
  project: Project;
  onClose: () => void;
  onSaved: (project: Project) => void;
}) {
  const [form, setForm] = useState({
    name: project.name,
    description: project.description,
    repo_path: project.repo_path,
    branch_dev: project.branch_dev,
    branch_main: project.branch_main,
  });
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState('');

  const submit = async () => {
    const name = form.name.trim();
    if (!name) { setError('项目名称不能为空'); return; }
    setLoading(true); setError('');
    try {
      const saved = await updateProject(project.id, {
        name,
        description: form.description.trim(),
        repo_path: form.repo_path.trim(),
        branch_dev: form.branch_dev.trim() || 'dev',
        branch_main: form.branch_main.trim() || 'main',
      });
      onSaved(saved);
    } catch (e) { setError(String(e)); }
    finally { setLoading(false); }
  };

  return (
    <div style={{ position: 'fixed', inset: 'var(--win-gutter,0)', borderRadius: 14, background: 'rgba(0,0,0,.5)', backdropFilter: 'blur(3px)', display: 'grid', placeItems: 'center', zIndex: 220 }}>
      <div style={{ width: 520, background: 'var(--bg-2)', border: '1px solid var(--border-strong)', borderRadius: 18, boxShadow: 'var(--shadow-lg)', overflow: 'hidden' }} onClick={e => e.stopPropagation()}>
        <div style={{ padding: '18px 20px 14px', borderBottom: '1px solid var(--border)', display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
          <div className="eyebrow" style={{ fontSize: 'var(--text-section)' }}><span className="cn">编辑项目</span> <span style={{ fontSize: 'var(--text-label)', color: 'var(--text-3)', fontWeight: 400 }}>{project.slug}</span></div>
          <button className="icon-btn" onClick={onClose}><Icon name="x" size={18} /></button>
        </div>
        <div style={{ padding: '16px 20px' }}>
          <ProjectFormFields form={form} setForm={setForm} slugTouched={false} setSlugTouched={() => {}} updateName={name => setForm(f => ({ ...f, name }))} showSlug={false} />
          {error && <div style={{ color: 'var(--red)', fontSize: 'var(--text-control)', marginTop: 10 }}>{error}</div>}
        </div>
        <div style={{ padding: '14px 20px', borderTop: '1px solid var(--border)', display: 'flex', gap: 10, justifyContent: 'flex-end' }}>
          <button className="btn" onClick={onClose}>取消</button>
          <button className="btn btn-primary" onClick={submit} disabled={loading}>
            <Icon name="check" size={15} />{loading ? '保存中...' : '保存'}
          </button>
        </div>
      </div>
    </div>
  );
}

// ── Shared form fields ────────────────────────────────────────────────────────

type FormState = {
  name: string; description: string; repo_path: string;
  branch_dev: string; branch_main: string;
  slug?: string;
};

function ProjectFormFields({
  form, setForm, slugTouched, setSlugTouched, updateName, showSlug,
  showRepoPath = true, repoPathLabel = '仓库路径', repoPathPlaceholder = '/home/user/project',
}: {
  form: FormState & { slug?: string };
  setForm: React.Dispatch<React.SetStateAction<any>>;
  slugTouched: boolean; setSlugTouched: (v: boolean) => void;
  updateName: (name: string) => void;
  showSlug: boolean;
  showRepoPath?: boolean;
  repoPathLabel?: string;
  repoPathPlaceholder?: string;
}) {
  return (
    <>
      <div className="cfg-fields" style={{ gridTemplateColumns: showSlug ? '1fr 1fr' : '1fr' }}>
        <div className="field">
          <label>项目名称</label>
          <input value={form.name} onChange={e => updateName(e.target.value)} placeholder="例如：AutoForge" />
        </div>
        {showSlug && (
          <div className="field">
            <label>项目标识</label>
            <input value={form.slug ?? ''} onChange={e => { setSlugTouched(true); setForm((f: any) => ({ ...f, slug: e.target.value })); }} placeholder="autoforge" />
          </div>
        )}
        {showRepoPath && (
          <div className={`field${showSlug ? ' full' : ''}`}>
            <label>{repoPathLabel}</label>
            <input value={form.repo_path} onChange={e => setForm((f: any) => ({ ...f, repo_path: e.target.value }))} placeholder={repoPathPlaceholder} />
          </div>
        )}
        <div className={`field${showSlug ? ' full' : ''}`}>
          <label>项目描述</label>
          <input value={form.description} onChange={e => setForm((f: any) => ({ ...f, description: e.target.value }))} placeholder="用于区分项目范围和业务目标" />
        </div>
        <div className="field"><label>开发分支</label><input value={form.branch_dev} onChange={e => setForm((f: any) => ({ ...f, branch_dev: e.target.value }))} /></div>
        <div className="field"><label>主分支</label><input value={form.branch_main} onChange={e => setForm((f: any) => ({ ...f, branch_main: e.target.value }))} /></div>
      </div>
      <div style={{ fontSize: 'var(--text-micro)', color: 'var(--text-faint)', marginTop: 9, lineHeight: 'var(--leading-normal)' }}>
        预览 / 测试 / 部署等运行配置在创建后于「运行配置」标签页维护。
      </div>
    </>
  );
}

// ── ConfirmModal (generic) ────────────────────────────────────────────────────

export function ConfirmModal({ msg, sub, okLabel = '确认', danger = true, onOk, onCancel }: {
  msg: string;
  sub?: string;
  okLabel?: string;
  danger?: boolean;
  onOk: () => void;
  onCancel: () => void;
}) {
  return (
    <div style={{ position: 'fixed', inset: 'var(--win-gutter,0)', borderRadius: 14, background: 'rgba(0,0,0,.5)', backdropFilter: 'blur(3px)', display: 'grid', placeItems: 'center', zIndex: 230 }}>
      <div style={{ background: 'var(--bg-2)', border: '1px solid var(--border-strong)', borderRadius: 14, padding: '22px 24px', width: 400, boxShadow: 'var(--shadow-lg)' }} onClick={e => e.stopPropagation()}>
        <p style={{ margin: sub ? '0 0 8px' : '0 0 20px', fontSize: 'var(--text-body)', lineHeight: 'var(--leading-relaxed)' }}>{msg}</p>
        {sub && <p style={{ margin: '0 0 20px', fontSize: 'var(--text-control)', lineHeight: 'var(--leading-relaxed)', color: 'var(--text-3)' }}>{sub}</p>}
        <div style={{ display: 'flex', gap: 10, justifyContent: 'flex-end' }}>
          <button className="btn" onClick={onCancel}>取消</button>
          <button className={`btn ${danger ? 'btn-danger' : 'btn-primary'}`} onClick={onOk}>{okLabel}</button>
        </div>
      </div>
    </div>
  );
}

// ── ConfirmProjectDeleteModal ─────────────────────────────────────────────────

export function ConfirmProjectDeleteModal({ project, onCancel, onConfirm }: {
  project: Project;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  return (
    <div style={{ position: 'fixed', inset: 'var(--win-gutter,0)', borderRadius: 14, background: 'rgba(0,0,0,.5)', backdropFilter: 'blur(3px)', display: 'grid', placeItems: 'center', zIndex: 230 }}>
      <div style={{ background: 'var(--bg-2)', border: '1px solid var(--border-strong)', borderRadius: 14, padding: '22px 24px', width: 420, boxShadow: 'var(--shadow-lg)' }} onClick={e => e.stopPropagation()}>
        <p style={{ margin: '0 0 8px', fontSize: 'var(--text-body)', lineHeight: 'var(--leading-relaxed)' }}>确认删除项目「{project.name}」？</p>
        <p style={{ margin: '0 0 20px', fontSize: 'var(--text-control)', lineHeight: 'var(--leading-relaxed)', color: 'var(--text-3)' }}>这会同时删除该项目的需求、变更请求、审核记录、预览环境和测试记录。</p>
        <div style={{ display: 'flex', gap: 10, justifyContent: 'flex-end' }}>
          <button className="btn" onClick={onCancel}>取消</button>
          <button className="btn btn-danger" onClick={onConfirm}><Icon name="trash" size={15} />确认删除</button>
        </div>
      </div>
    </div>
  );
}
