import React, { useState } from 'react';
import Icon from './Icon';
import { createProject, type Project } from '../services';

const slugify = (name: string) => {
  const slug = name
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '');
  return slug || `project-${Date.now().toString(36)}`;
};

export function ProjectCreateModal({ onClose, onCreated }: {
  onClose: () => void;
  onCreated: (project: Project) => void;
}) {
  const [form, setForm] = useState({
    name: '',
    slug: '',
    description: '',
    repo_path: '',
    branch_dev: 'dev',
    branch_main: 'main',
  });
  const [slugTouched, setSlugTouched] = useState(false);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState('');

  const updateName = (name: string) => {
    setForm(f => ({ ...f, name, slug: slugTouched ? f.slug : slugify(name) }));
  };

  const submit = async () => {
    const name = form.name.trim();
    const slug = form.slug.trim();
    const repoPath = form.repo_path.trim();
    if (!name) { setError('项目名称不能为空'); return; }
    if (!slug) { setError('项目标识不能为空'); return; }
    if (!repoPath) { setError('仓库路径不能为空'); return; }

    setLoading(true);
    setError('');
    try {
      const project = await createProject({
        name,
        slug,
        description: form.description.trim(),
        repo_path: repoPath,
        branch_dev: form.branch_dev.trim() || 'dev',
        branch_main: form.branch_main.trim() || 'main',
      });
      onCreated(project);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  };

  return (
    <div style={{ position: 'fixed', inset: 0, background: 'rgba(0,0,0,.5)', backdropFilter: 'blur(3px)', display: 'grid', placeItems: 'center', zIndex: 220 }} onClick={onClose}>
      <div style={{ width: 520, background: 'var(--bg-2)', border: '1px solid var(--border-strong)', borderRadius: 18, boxShadow: 'var(--shadow-lg)', overflow: 'hidden' }} onClick={e => e.stopPropagation()}>
        <div style={{ padding: '18px 20px 14px', borderBottom: '1px solid var(--border)', display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
          <div className="eyebrow" style={{ fontSize: 16 }}><span className="cn">添加项目</span></div>
          <button className="icon-btn" onClick={onClose}><Icon name="x" size={18} /></button>
        </div>
        <div style={{ padding: '16px 20px' }}>
          <div className="cfg-fields" style={{ gridTemplateColumns: '1fr 1fr' }}>
            <div className="field"><label>项目名称</label><input value={form.name} onChange={e => updateName(e.target.value)} placeholder="例如：AutoForge" /></div>
            <div className="field"><label>项目标识</label><input value={form.slug} onChange={e => { setSlugTouched(true); setForm(f => ({ ...f, slug: e.target.value })); }} placeholder="autoforge" /></div>
            <div className="field full"><label>仓库路径</label><input value={form.repo_path} onChange={e => setForm(f => ({ ...f, repo_path: e.target.value }))} placeholder="/home/user/project" /></div>
            <div className="field full"><label>项目描述</label><input value={form.description} onChange={e => setForm(f => ({ ...f, description: e.target.value }))} placeholder="用于区分项目范围和业务目标" /></div>
            <div className="field"><label>开发分支</label><input value={form.branch_dev} onChange={e => setForm(f => ({ ...f, branch_dev: e.target.value }))} /></div>
            <div className="field"><label>主分支</label><input value={form.branch_main} onChange={e => setForm(f => ({ ...f, branch_main: e.target.value }))} /></div>
          </div>
          {error && <div style={{ color: 'var(--red)', fontSize: 13, marginTop: 10 }}>{error}</div>}
        </div>
        <div style={{ padding: '14px 20px', borderTop: '1px solid var(--border)', display: 'flex', gap: 10, justifyContent: 'flex-end' }}>
          <button className="btn" onClick={onClose}>取消</button>
          <button className="btn btn-primary" onClick={submit} disabled={loading}>
            <Icon name="plus" size={15} />{loading ? '添加中...' : '添加项目'}
          </button>
        </div>
      </div>
    </div>
  );
}

export function ConfirmProjectDeleteModal({ project, onCancel, onConfirm }: {
  project: Project;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  return (
    <div style={{ position: 'fixed', inset: 0, background: 'rgba(0,0,0,.5)', backdropFilter: 'blur(3px)', display: 'grid', placeItems: 'center', zIndex: 230 }} onClick={onCancel}>
      <div style={{ background: 'var(--bg-2)', border: '1px solid var(--border-strong)', borderRadius: 14, padding: '22px 24px', width: 420, boxShadow: 'var(--shadow-lg)' }} onClick={e => e.stopPropagation()}>
        <p style={{ margin: '0 0 8px', fontSize: 14, lineHeight: 1.6 }}>确认删除项目「{project.name}」？</p>
        <p style={{ margin: '0 0 20px', fontSize: 12.5, lineHeight: 1.6, color: 'var(--text-3)' }}>这会同时删除该项目的需求、变更请求、审核记录、预览环境和测试记录。</p>
        <div style={{ display: 'flex', gap: 10, justifyContent: 'flex-end' }}>
          <button className="btn" onClick={onCancel}>取消</button>
          <button className="btn btn-danger" onClick={onConfirm}><Icon name="trash" size={15} />确认删除</button>
        </div>
      </div>
    </div>
  );
}
