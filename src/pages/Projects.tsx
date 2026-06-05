import React, { useState, useEffect, useCallback, useRef } from 'react';
import { listen } from '@tauri-apps/api/event';
import Icon from '../components/Icon';
import Select from '../components/Select';
import { ProjectCreateModal, ProjectEditModal, ConfirmProjectDeleteModal } from '../components/ProjectDialogs';
import {
  listProjects, updateProject, deleteProject, type Project,
  listMaterialFolders, createMaterialFolder, renameMaterialFolder, deleteMaterialFolder,
  listMaterialFiles, importMaterialFile, moveMaterialFile, deleteMaterialFile,
  openMaterialFile, aiOrganizeMaterials, backupMaterialFiles,
  getMaterialBackupConfig, updateMaterialBackupConfig,
  getIntakeConfig, updateIntakeConfig, syncGithubIssues, runCodeScan, bulkImportIssues,
  listProjectSpecs, upsertProjectSpec, deleteProjectSpec, aiGenerateSpecs,
  type MaterialFolder, type MaterialFile, type MaterialBackupConfig,
  type IntakeConfig, type SyncResult, type ScanResult, type BulkResult,
  type ProjectSpec, type SpecCategory,
} from '../services';

// ── helpers ───────────────────────────────────────────────────────────────────

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

function formatMaterialTime(value: string): string {
  const d = new Date(value);
  if (Number.isNaN(d.getTime())) return '—';
  return d.toLocaleString('zh-CN', {
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
  });
}

function fileIcon(mime: string): string {
  if (mime.startsWith('image/')) return 'image';
  if (mime.includes('spreadsheet') || mime.includes('excel') || mime.includes('csv')) return 'grid';
  if (mime.includes('zip') || mime.includes('tar') || mime.includes('compressed')) return 'package';
  return 'file';
}

function buildTree(folders: MaterialFolder[], parentId: string | null): MaterialFolder[] {
  return folders
    .filter(f => f.parent_id === parentId)
    .sort((a, b) => a.sort_order - b.sort_order || a.name.localeCompare(b.name));
}

function findDocsRoot(folders: MaterialFolder[]): MaterialFolder | null {
  return folders.find(f => f.parent_id === null && f.name.toLowerCase() === 'docs') ?? null;
}

function visibleMaterialFolders(folders: MaterialFolder[]): MaterialFolder[] {
  const docsRoot = findDocsRoot(folders);
  return docsRoot ? folders.filter(f => f.id !== docsRoot.id) : folders;
}

function countFilesInFolderTree(folderId: string, folders: MaterialFolder[], files: MaterialFile[]): number {
  const childIds = folders.filter(f => f.parent_id === folderId).map(f => f.id);
  return files.filter(f => f.folder_id === folderId).length
    + childIds.reduce((sum, id) => sum + countFilesInFolderTree(id, folders, files), 0);
}

// ── BackupConfigModal ─────────────────────────────────────────────────────────

function BackupConfigModal({ config, onSave, onClose }: {
  config: MaterialBackupConfig;
  onSave: (c: MaterialBackupConfig) => void;
  onClose: () => void;
}) {
  const parsed = (() => { try { return JSON.parse(config.config_json); } catch { return {}; } })();
  const [provider, setProvider]   = useState(config.provider);
  const [enabled,  setEnabled]    = useState(config.enabled);
  const [localPath, setLocalPath] = useState(parsed.path ?? '');
  const [remote, setRemote]       = useState(parsed.remote ?? '');
  const [rclonePath, setRclonePath] = useState(parsed.path ?? '');
  const [saving, setSaving] = useState(false);
  const [err, setErr] = useState('');

  const save = async () => {
    setSaving(true); setErr('');
    try {
      let cfgJson = '{}';
      if (provider === 'local')  cfgJson = JSON.stringify({ path: localPath });
      if (provider === 'rclone') cfgJson = JSON.stringify({ remote, path: rclonePath });
      onSave(await updateMaterialBackupConfig(provider, cfgJson, enabled));
    } catch (e) { setErr(String(e)); }
    finally { setSaving(false); }
  };

  return (
    <div style={{ position: 'fixed', inset: 0, background: 'rgba(0,0,0,.5)', backdropFilter: 'blur(3px)', display: 'grid', placeItems: 'center', zIndex: 220 }} onClick={onClose}>
      <div style={{ width: 480, background: 'var(--bg-2)', border: '1px solid var(--border-strong)', borderRadius: 18, boxShadow: 'var(--shadow-lg)', padding: '22px 24px' }} onClick={e => e.stopPropagation()}>
        <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 20 }}>
          <div className="eyebrow" style={{ fontSize: 'var(--text-body)' }}><span className="en">BACKUP</span><span className="cn"> · 云存储备份</span></div>
          <button className="icon-btn" onClick={onClose}><Icon name="x" size={16} /></button>
        </div>

        {err && <div style={{ color: 'var(--red)', fontSize: 'var(--text-label)', marginBottom: 12, padding: '6px 10px', background: 'rgba(219,90,64,.08)', borderRadius: 6 }}>{err}</div>}

        <div style={{ fontSize: 'var(--text-label)', color: 'var(--text-3)', marginBottom: 8 }}>存储提供商</div>
        <div style={{ display: 'flex', gap: 8, marginBottom: 18 }}>
          {(['none', 'local', 'rclone'] as const).map(p => (
            <button key={p} className={'btn btn-sm' + (provider === p ? ' btn-primary' : '')} onClick={() => setProvider(p)}>
              {p === 'none' ? '不备份' : p === 'local' ? '本地目录' : 'Rclone 云盘'}
            </button>
          ))}
        </div>

        {provider === 'local' && (
          <div className="field" style={{ marginBottom: 16 }}>
            <label>备份目标目录</label>
            <input placeholder="/home/user/backup/autoforge" value={localPath} onChange={e => setLocalPath(e.target.value)} />
          </div>
        )}

        {provider === 'rclone' && (<>
          <div className="field" style={{ marginBottom: 12 }}>
            <label>Rclone Remote 名称</label>
            <input placeholder="gdrive" value={remote} onChange={e => setRemote(e.target.value)} />
          </div>
          <div className="field" style={{ marginBottom: 12 }}>
            <label>云端路径前缀</label>
            <input placeholder="AutoForge/materials" value={rclonePath} onChange={e => setRclonePath(e.target.value)} />
          </div>
          <div style={{ fontSize: 'var(--text-caption)', color: 'var(--text-faint)', marginBottom: 16, lineHeight: 'var(--leading-normal)' }}>
            需预先安装并配置 rclone（<code>rclone config</code>），支持 Google Drive、S3、Dropbox、OneDrive 等。
          </div>
        </>)}

        {provider !== 'none' && (
          <label style={{ display: 'flex', alignItems: 'center', gap: 8, fontSize: 'var(--text-control)', cursor: 'pointer', marginBottom: 20 }}>
            <input type="checkbox" checked={enabled} onChange={e => setEnabled(e.target.checked)} />
            启用自动备份
          </label>
        )}

        <div style={{ display: 'flex', justifyContent: 'flex-end', gap: 8 }}>
          <button className="btn btn-sm" onClick={onClose}>取消</button>
          <button className="btn btn-sm btn-primary" onClick={save} disabled={saving}>{saving ? '保存中…' : '保存配置'}</button>
        </div>
      </div>
    </div>
  );
}

// ── FolderTreeItem ────────────────────────────────────────────────────────────

function FolderTreeItem({ folder, allFolders, allFiles, depth, selectedId, onSelect, onCreate, onRename, onDelete }: {
  folder: MaterialFolder; allFolders: MaterialFolder[]; allFiles: MaterialFile[];
  depth: number; selectedId: string | null;
  onSelect: (id: string) => void; onCreate: (parentId: string) => void;
  onRename: (f: MaterialFolder) => void; onDelete: (f: MaterialFolder) => void;
}) {
  const [expanded, setExpanded] = useState(true);
  const [hovered, setHovered]   = useState(false);
  const children = buildTree(allFolders, folder.id);
  const count = countFilesInFolderTree(folder.id, allFolders, allFiles);
  const active = selectedId === folder.id;

  return (
    <div>
      <div
        style={{
          display: 'flex', alignItems: 'center', gap: 4,
          padding: `4px 6px 4px ${6 + depth * 14}px`,
          borderRadius: 6, cursor: 'pointer',
          background: active ? 'var(--ember-tint)' : hovered ? 'var(--surface-hover)' : 'transparent',
          color: active ? 'var(--ember)' : 'var(--text-2)',
          fontSize: 'var(--text-control)', userSelect: 'none',
        }}
        onMouseEnter={() => setHovered(true)}
        onMouseLeave={() => setHovered(false)}
        onClick={() => onSelect(folder.id)}
      >
        {children.length > 0
          ? <button style={{ background: 'none', border: 'none', padding: 0, cursor: 'pointer', color: 'inherit', flexShrink: 0, lineHeight: 1 }} onClick={e => { e.stopPropagation(); setExpanded(v => !v); }}>
              <Icon name={expanded ? 'chevDown' : 'chevRight'} size={10} />
            </button>
          : <div style={{ width: 10, flexShrink: 0 }} />
        }
        <Icon name={active ? 'folderOpen' : 'folder'} size={13} style={{ flexShrink: 0 }} />
        <span style={{ flex: 1, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{folder.name}</span>
        {count > 0 && <span style={{ fontSize: 'var(--text-micro)', color: 'var(--text-faint)', flexShrink: 0 }}>{count}</span>}
        {hovered && (
          <div style={{ display: 'flex', gap: 1, flexShrink: 0 }} onClick={e => e.stopPropagation()}>
            <button className="btn btn-sm" style={{ padding: '1px 3px' }} onClick={() => onCreate(folder.id)} title="新建子文件夹"><Icon name="plus" size={10} /></button>
            <button className="btn btn-sm" style={{ padding: '1px 3px' }} onClick={() => onRename(folder)} title="重命名"><Icon name="edit" size={10} /></button>
            <button className="btn btn-sm" style={{ padding: '1px 3px', color: 'var(--red)' }} onClick={() => onDelete(folder)} title="删除"><Icon name="trash" size={10} /></button>
          </div>
        )}
      </div>
      {expanded && children.map(c => (
        <FolderTreeItem key={c.id} folder={c} allFolders={allFolders} allFiles={allFiles} depth={depth + 1}
          selectedId={selectedId} onSelect={onSelect} onCreate={onCreate} onRename={onRename} onDelete={onDelete} />
      ))}
    </div>
  );
}

// ── FileCard ──────────────────────────────────────────────────────────────────

function FileCard({ file, onOpen, onDelete, onMove }: {
  file: MaterialFile;
  onOpen: () => void; onDelete: () => void; onMove: () => void;
}) {
  const [hovered, setHovered] = useState(false);
  const updatedTitle = new Date(file.updated_at).toLocaleString('zh-CN');
  const backupChip = file.backup_status === 'synced'
    ? <span className="chip green" style={{ fontSize: 'var(--text-micro)', padding: '1px 5px' }}>已备份</span>
    : file.backup_status === 'error'
    ? <span className="chip red" style={{ fontSize: 'var(--text-micro)', padding: '1px 5px' }}>备份失败</span>
    : null;

  return (
    <div
      style={{
        background: 'var(--bg-2)', border: `1px solid ${hovered ? 'var(--border-strong)' : 'var(--border)'}`,
        borderRadius: 10, padding: '9px 10px 9px 12px', minHeight: 54,
        display: 'grid', gridTemplateColumns: '34px minmax(180px, 1fr) minmax(110px, auto) 104px auto',
        alignItems: 'center', gap: 11,
        cursor: 'default', transition: 'border-color .15s, transform .15s',
      }}
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
    >
      <div style={{ width: 34, height: 34, borderRadius: 8, flexShrink: 0, background: 'var(--bg-3)', display: 'flex', alignItems: 'center', justifyContent: 'center', color: 'var(--text-3)' }}>
        <Icon name={fileIcon(file.mime)} size={17} />
      </div>
      <div style={{ minWidth: 0 }}>
        <div title={file.original_name} style={{ fontSize: 'var(--text-control)', fontWeight: 650, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
          {file.original_name}
        </div>
        <div style={{ fontSize: 'var(--text-caption)', color: 'var(--text-3)', marginTop: 2, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
          {file.description || file.mime}
        </div>
      </div>
      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'flex-end', gap: 8, minWidth: 110 }}>
        <span style={{ fontSize: 'var(--text-label)', color: 'var(--text-faint)', fontFamily: 'var(--font-mono)', whiteSpace: 'nowrap' }}>{formatSize(file.size_bytes)}</span>
        {backupChip}
      </div>
      <div title={`最后操作：${updatedTitle}`} style={{ fontSize: 'var(--text-label)', color: 'var(--text-3)', fontFamily: 'var(--font-mono)', whiteSpace: 'nowrap', textAlign: 'right' }}>
        {formatMaterialTime(file.updated_at)}
      </div>
      <div
        style={{
          display: 'flex', gap: 5, opacity: hovered ? 1 : .62,
          transition: 'opacity .14s',
        }}
      >
        <button className="icon-btn" style={{ width: 30, height: 30 }} onClick={onOpen} title="打开" aria-label="打开">
          <Icon name="external" size={14} />
        </button>
        <button className="icon-btn" style={{ width: 30, height: 30 }} onClick={onMove} title="移动" aria-label="移动">
          <Icon name="moveFile" size={14} />
        </button>
        <button className="icon-btn" style={{ width: 30, height: 30, color: 'var(--red)' }} onClick={onDelete} title="删除" aria-label="删除">
          <Icon name="trash" size={14} />
        </button>
      </div>
    </div>
  );
}

// ── MoveFileModal ─────────────────────────────────────────────────────────────

function MoveFileModal({ file, folders, onMove, onClose }: {
  file: MaterialFile; folders: MaterialFolder[];
  onMove: (folderId: string | null) => void; onClose: () => void;
}) {
  const docsRoot = findDocsRoot(folders);
  const visibleFolders = visibleMaterialFolders(folders);
  return (
    <div style={{ position: 'fixed', inset: 0, background: 'rgba(0,0,0,.5)', backdropFilter: 'blur(3px)', display: 'grid', placeItems: 'center', zIndex: 220 }} onClick={onClose}>
      <div style={{ width: 360, background: 'var(--bg-2)', border: '1px solid var(--border-strong)', borderRadius: 18, boxShadow: 'var(--shadow-lg)', padding: '20px 24px' }} onClick={e => e.stopPropagation()}>
        <div style={{ fontWeight: 600, marginBottom: 14 }}>移动到文件夹</div>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 4, maxHeight: 300, overflowY: 'auto' }}>
          <button className="btn btn-sm" style={{ justifyContent: 'flex-start', gap: 8, background: file.folder_id === null || file.folder_id === docsRoot?.id ? 'var(--ember-tint)' : '' }} onClick={() => onMove(null)}>
            <Icon name="layers" size={13} />根目录
          </button>
          {visibleFolders.map(f => (
            <button key={f.id} className="btn btn-sm" style={{ justifyContent: 'flex-start', gap: 8, background: file.folder_id === f.id ? 'var(--ember-tint)' : '' }} onClick={() => onMove(f.id)}>
              <Icon name="folder" size={13} />{f.name}
            </button>
          ))}
        </div>
        <div style={{ display: 'flex', justifyContent: 'flex-end', marginTop: 16 }}>
          <button className="btn btn-sm" onClick={onClose}>取消</button>
        </div>
      </div>
    </div>
  );
}

// ── MaterialsPanel ────────────────────────────────────────────────────────────

function MaterialsPanel({ projectId }: { projectId: string }) {
  const [folders, setFolders] = useState<MaterialFolder[]>([]);
  const [files, setFiles]     = useState<MaterialFile[]>([]);
  const [selectedFolderId, setSelectedFolderId] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError]   = useState('');
  const [message, setMessage] = useState('');

  const [aiWorking,     setAiWorking]     = useState(false);
  const [backupWorking, setBackupWorking] = useState(false);
  const [uploading,     setUploading]     = useState(false);
  const [refreshing,    setRefreshing]    = useState(false);

  const [showBackupConfig, setShowBackupConfig] = useState(false);
  const [backupConfig, setBackupConfig]         = useState<MaterialBackupConfig | null>(null);

  const [creatingFolder, setCreatingFolder] = useState<{ parentId: string | null } | null>(null);
  const [newFolderName, setNewFolderName]   = useState('');
  const [renamingFolder, setRenamingFolder] = useState<MaterialFolder | null>(null);
  const [renameName, setRenameName]         = useState('');
  const [movingFile, setMovingFile]         = useState<MaterialFile | null>(null);
  const [dragOver, setDragOver]             = useState(false);

  const fileInputRef = useRef<HTMLInputElement>(null);
  const docsRoot = findDocsRoot(folders);
  const visibleFolders = visibleMaterialFolders(folders);
  const displayedFiles = selectedFolderId === null
    ? files.filter(f => docsRoot ? f.folder_id === docsRoot.id : f.folder_id === null)
    : files.filter(f => f.folder_id === selectedFolderId);
  const rootFileCount = files.filter(f => docsRoot ? f.folder_id === docsRoot.id : f.folder_id === null).length;

  const load = useCallback(async () => {
    try {
      const flds = await listMaterialFolders(projectId);
      const fils = await listMaterialFiles(projectId);
      setFolders(flds);
      setFiles(fils);
    } catch (e) { setError(String(e)); }
    finally { setLoading(false); }
  }, [projectId]);

  useEffect(() => {
    setSelectedFolderId(null);
    setLoading(true);
    load();
  }, [load]);
  useEffect(() => { getMaterialBackupConfig().then(setBackupConfig).catch(() => {}); }, []);

  useEffect(() => {
    const id = setInterval(() => { load(); }, 10_000);
    return () => clearInterval(id);
  }, [load]);

  const flash = (msg: string) => { setMessage(msg); setTimeout(() => setMessage(''), 4000); };

  const handleFiles = async (fileList: FileList) => {
    setUploading(true); setError('');
    for (const f of Array.from(fileList)) {
      try {
        const buf = await f.arrayBuffer();
        const b64 = btoa(String.fromCharCode(...new Uint8Array(buf)));
        await importMaterialFile(projectId, selectedFolderId, f.name, f.type, b64);
      } catch (e) { setError(String(e)); }
    }
    setUploading(false);
    await load();
  };

  const doCreateFolder = async () => {
    if (!newFolderName.trim()) return;
    try {
      await createMaterialFolder(projectId, creatingFolder?.parentId ?? null, newFolderName.trim());
      setCreatingFolder(null); setNewFolderName('');
      await load();
    } catch (e) { setError(String(e)); }
  };

  const doRenameFolder = async () => {
    if (!renamingFolder || !renameName.trim()) return;
    try {
      await renameMaterialFolder(renamingFolder.id, renameName.trim());
      setRenamingFolder(null); setRenameName('');
      await load();
    } catch (e) { setError(String(e)); }
  };

  const doDeleteFolder = async (f: MaterialFolder) => {
    if (!confirm(`确认删除文件夹「${f.name}」？文件夹内的文件将移至根目录。`)) return;
    try {
      const ok = await deleteMaterialFolder(f.id);
      if (!ok) throw new Error('文件夹不存在或已被删除');
      if (selectedFolderId === f.id) setSelectedFolderId(null);
      await load();
    } catch (e) { setError(String(e)); }
  };

  const doDeleteFile = async (f: MaterialFile) => {
    if (!confirm(`确认删除「${f.original_name}」？此操作不可撤销。`)) return;
    try { await deleteMaterialFile(f.id); await load(); }
    catch (e) { setError(String(e)); }
  };

  const doMoveFile = async (folderId: string | null) => {
    if (!movingFile) return;
    try { await moveMaterialFile(movingFile.id, folderId); setMovingFile(null); await load(); }
    catch (e) { setError(String(e)); }
  };

  const doRefresh = async () => {
    setRefreshing(true); setError('');
    try { await load(); }
    catch (e) { setError(String(e)); }
    finally { setRefreshing(false); }
  };

  const doAiOrganize = async () => {
    setAiWorking(true); setError('');
    try { flash(await aiOrganizeMaterials(projectId)); await load(); }
    catch (e) { setError(String(e)); }
    finally { setAiWorking(false); }
  };

  const doBackup = async () => {
    setBackupWorking(true); setError('');
    try { flash(await backupMaterialFiles(projectId, [])); await load(); }
    catch (e) { setError(String(e)); }
    finally { setBackupWorking(false); }
  };

  const rootFolders = docsRoot ? buildTree(folders, docsRoot.id) : buildTree(visibleFolders, null);

  return (
    <div style={{ flex: 1, display: 'flex', flexDirection: 'column', overflow: 'hidden', minHeight: 0 }}>

      {/* action bar */}
      <div style={{ display: 'flex', alignItems: 'center', gap: 6, padding: '10px 20px', borderBottom: '1px solid var(--border)', flexShrink: 0, flexWrap: 'wrap' }}>
        <input ref={fileInputRef} type="file" multiple style={{ display: 'none' }} onChange={e => { if (e.target.files?.length) handleFiles(e.target.files); e.target.value = ''; }} />
        <button className="btn btn-sm btn-primary" onClick={() => fileInputRef.current?.click()} disabled={uploading}>
          <Icon name="upload" size={13} />{uploading ? '上传中…' : '上传文件'}
        </button>
        <button className="btn btn-sm" onClick={() => { setCreatingFolder({ parentId: selectedFolderId }); setNewFolderName(''); }}>
          <Icon name="folderPlus" size={13} />新建文件夹
        </button>
        <button className="btn btn-sm" onClick={doRefresh} disabled={refreshing} title="刷新文件列表">
          <Icon name="refresh" size={13} style={refreshing ? { animation: 'spin 1s linear infinite' } : undefined} />{refreshing ? '刷新中…' : '刷新'}
        </button>
        <div style={{ flex: 1 }} />
        {message && <span style={{ fontSize: 'var(--text-label)', color: 'var(--text-3)', maxWidth: 260, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{message}</span>}
        <button className="btn btn-sm" onClick={doAiOrganize} disabled={aiWorking}>
          <Icon name="brain" size={13} />{aiWorking ? 'AI 整理中…' : 'AI 整理'}
        </button>
        <button className="btn btn-sm" onClick={doBackup} disabled={backupWorking}>
          <Icon name="cloudUpload" size={13} />{backupWorking ? '备份中…' : '批量备份'}
        </button>
        <button className="btn btn-sm" onClick={() => setShowBackupConfig(true)} title="备份配置">
          <Icon name="settings" size={13} />
        </button>
      </div>

      {error && (
        <div style={{ padding: '6px 20px', color: 'var(--red)', fontSize: 'var(--text-label)', background: 'rgba(219,90,64,.06)', flexShrink: 0 }}>{error}</div>
      )}

      {/* body: folder sidebar + file grid */}
      <div style={{ flex: 1, display: 'flex', overflow: 'hidden', minHeight: 0 }}>

        {/* folder sidebar */}
        <div style={{ width: 190, flexShrink: 0, borderRight: '1px solid var(--border)', overflowY: 'auto', padding: '10px 6px' }}>
          {/* "all" root */}
          <div
            style={{ display: 'flex', alignItems: 'center', gap: 6, padding: '4px 8px', borderRadius: 6, cursor: 'pointer', fontSize: 'var(--text-control)', marginBottom: 2, background: selectedFolderId === null ? 'var(--ember-tint)' : 'transparent', color: selectedFolderId === null ? 'var(--ember)' : 'var(--text-2)' }}
            onClick={() => setSelectedFolderId(null)}
          >
            <Icon name="layers" size={13} />
            <span style={{ flex: 1 }}>根目录</span>
            <span style={{ fontSize: 'var(--text-micro)', color: 'var(--text-faint)' }}>{rootFileCount}</span>
          </div>

          {loading
            ? <div style={{ fontSize: 'var(--text-caption)', color: 'var(--text-faint)', padding: '8px 8px' }}>加载中…</div>
            : rootFolders.map(f => (
                <FolderTreeItem key={f.id} folder={f} allFolders={visibleFolders} allFiles={files}
                  depth={0} selectedId={selectedFolderId}
                  onSelect={setSelectedFolderId}
                  onCreate={parentId => { setCreatingFolder({ parentId }); setNewFolderName(''); }}
                  onRename={f => { setRenamingFolder(f); setRenameName(f.name); }}
                  onDelete={doDeleteFolder}
                />
              ))
          }

          {/* inline root folder creator */}
          {creatingFolder !== null && creatingFolder.parentId === null && (
            <div style={{ padding: '6px 4px', marginTop: 4 }}>
              <div className="field">
                <input autoFocus placeholder="文件夹名称" value={newFolderName}
                  onChange={e => setNewFolderName(e.target.value)}
                  onKeyDown={e => { if (e.key === 'Enter') doCreateFolder(); if (e.key === 'Escape') setCreatingFolder(null); }} />
              </div>
              <div style={{ display: 'flex', gap: 4, marginTop: 5 }}>
                <button className="btn btn-sm btn-primary" style={{ flex: 1, fontSize: 'var(--text-caption)' }} onClick={doCreateFolder}>确认</button>
                <button className="btn btn-sm" style={{ flex: 1, fontSize: 'var(--text-caption)' }} onClick={() => setCreatingFolder(null)}>取消</button>
              </div>
            </div>
          )}
        </div>

        {/* file area */}
        <div
          style={{ flex: 1, overflowY: 'auto', padding: '14px 16px', background: dragOver ? 'rgba(232,119,46,.04)' : 'transparent', transition: 'background .15s', position: 'relative' }}
          onDragOver={e => { e.preventDefault(); setDragOver(true); }}
          onDragLeave={() => setDragOver(false)}
          onDrop={e => { e.preventDefault(); setDragOver(false); if (e.dataTransfer.files?.length) handleFiles(e.dataTransfer.files); }}
        >
          {loading ? (
            <div style={{ color: 'var(--text-3)', fontSize: 'var(--text-control)' }}>加载中…</div>
          ) : displayedFiles.length === 0 ? (
            <div className="empty" style={{ minHeight: 160 }}>
              <Icon name="upload" size={34} style={{ opacity: .25 }} />
              <div>拖放文件到此处，或点击上传</div>
              <button className="btn btn-sm" onClick={() => fileInputRef.current?.click()}>
                <Icon name="plus" size={12} />选择文件
              </button>
            </div>
          ) : (
            <div style={{ display: 'flex', flexDirection: 'column', gap: 7, minWidth: 0 }}>
              {displayedFiles.map(f => (
                <FileCard key={f.id} file={f}
                  onOpen={() => openMaterialFile(f.id).catch(e => setError(String(e)))}
                  onDelete={() => doDeleteFile(f)}
                  onMove={() => setMovingFile(f)}
                />
              ))}
            </div>
          )}

          {dragOver && (
            <div style={{ position: 'absolute', inset: 8, display: 'flex', alignItems: 'center', justifyContent: 'center', pointerEvents: 'none', background: 'rgba(232,119,46,.06)', borderRadius: 10, border: '2px dashed var(--ember)', fontSize: 'var(--text-title)', color: 'var(--ember)', fontWeight: 600 }}>
              松开鼠标上传文件
            </div>
          )}
        </div>
      </div>

      {/* rename folder modal */}
      {renamingFolder && (
        <div style={{ position: 'fixed', inset: 0, background: 'rgba(0,0,0,.5)', backdropFilter: 'blur(3px)', display: 'grid', placeItems: 'center', zIndex: 220 }} onClick={() => setRenamingFolder(null)}>
          <div style={{ width: 340, background: 'var(--bg-2)', border: '1px solid var(--border-strong)', borderRadius: 18, boxShadow: 'var(--shadow-lg)', padding: '20px 24px' }} onClick={e => e.stopPropagation()}>
            <div style={{ fontWeight: 600, marginBottom: 12 }}>重命名文件夹</div>
            <div className="field" style={{ marginBottom: 12 }}>
              <input autoFocus value={renameName} onChange={e => setRenameName(e.target.value)}
                onKeyDown={e => { if (e.key === 'Enter') doRenameFolder(); if (e.key === 'Escape') setRenamingFolder(null); }} />
            </div>
            <div style={{ display: 'flex', gap: 8, justifyContent: 'flex-end' }}>
              <button className="btn btn-sm" onClick={() => setRenamingFolder(null)}>取消</button>
              <button className="btn btn-sm btn-primary" onClick={doRenameFolder}>确认</button>
            </div>
          </div>
        </div>
      )}

      {/* create subfolder modal */}
      {creatingFolder !== null && creatingFolder.parentId !== null && (
        <div style={{ position: 'fixed', inset: 0, background: 'rgba(0,0,0,.5)', backdropFilter: 'blur(3px)', display: 'grid', placeItems: 'center', zIndex: 220 }} onClick={() => setCreatingFolder(null)}>
          <div style={{ width: 340, background: 'var(--bg-2)', border: '1px solid var(--border-strong)', borderRadius: 18, boxShadow: 'var(--shadow-lg)', padding: '20px 24px' }} onClick={e => e.stopPropagation()}>
            <div style={{ fontWeight: 600, marginBottom: 12 }}>新建子文件夹</div>
            <div className="field" style={{ marginBottom: 12 }}>
              <input autoFocus placeholder="文件夹名称" value={newFolderName}
                onChange={e => setNewFolderName(e.target.value)}
                onKeyDown={e => { if (e.key === 'Enter') doCreateFolder(); if (e.key === 'Escape') setCreatingFolder(null); }} />
            </div>
            <div style={{ display: 'flex', gap: 8, justifyContent: 'flex-end' }}>
              <button className="btn btn-sm" onClick={() => setCreatingFolder(null)}>取消</button>
              <button className="btn btn-sm btn-primary" onClick={doCreateFolder}>创建</button>
            </div>
          </div>
        </div>
      )}

      {movingFile && <MoveFileModal file={movingFile} folders={folders} onMove={doMoveFile} onClose={() => setMovingFile(null)} />}

      {showBackupConfig && backupConfig && (
        <BackupConfigModal config={backupConfig}
          onSave={cfg => { setBackupConfig(cfg); setShowBackupConfig(false); flash('备份配置已保存'); }}
          onClose={() => setShowBackupConfig(false)} />
      )}
    </div>
  );
}

// ── Intake helpers ────────────────────────────────────────────────────────────

function ICard({ children, style }: { children: React.ReactNode; style?: React.CSSProperties }) {
  return (
    <div style={{ background: 'var(--bg-2)', border: '1px solid var(--border-strong)', borderRadius: 14, padding: '18px 20px', ...style }}>
      {children}
    </div>
  );
}

function ISectionLabel({ children }: { children: React.ReactNode }) {
  return (
    <div style={{ fontSize: 'var(--text-caption)', fontWeight: 700, letterSpacing: '.07em', textTransform: 'uppercase', color: 'var(--text-faint)', marginBottom: 10 }}>
      {children}
    </div>
  );
}

function IResultBanner({ ok, children }: { ok?: boolean; children: React.ReactNode }) {
  return (
    <div style={{
      background: ok === false ? 'rgba(219,90,64,.1)' : 'rgba(79,157,107,.1)',
      border: `1px solid ${ok === false ? 'rgba(219,90,64,.3)' : 'rgba(79,157,107,.3)'}`,
      borderRadius: 10, padding: '10px 14px', fontSize: 'var(--text-control)',
      color: ok === false ? 'var(--red)' : 'var(--green)',
      display: 'flex', alignItems: 'flex-start', gap: 8,
    }}>
      <Icon name={ok === false ? 'alert' : 'check'} size={14} style={{ flexShrink: 0, marginTop: 1 }} />
      <div>{children}</div>
    </div>
  );
}

// ── ProjectGithubTab ──────────────────────────────────────────────────────────

function ProjectGithubTab({ projectId, cfg, onCfgChange }: {
  projectId: string;
  cfg: IntakeConfig;
  onCfgChange: (c: IntakeConfig) => void;
}) {
  const [form, setForm] = useState({ owner: cfg.github_owner, repo: cfg.github_repo, token: cfg.github_token });
  const [saving, setSaving]       = useState(false);
  const [syncing, setSyncing]     = useState(false);
  const [syncResult, setSyncResult] = useState<SyncResult | null>(null);
  const [syncErr, setSyncErr]     = useState('');
  const [saveOk, setSaveOk]       = useState<boolean | null>(null);

  const save = async () => {
    setSaving(true); setSaveOk(null);
    try {
      const updated = await updateIntakeConfig({
        github_owner: form.owner.trim(),
        github_repo: form.repo.trim(),
        github_token: form.token.trim(),
        github_project_id: projectId,
      });
      onCfgChange(updated);
      setSaveOk(true);
      setTimeout(() => setSaveOk(null), 2500);
    } catch { setSaveOk(false); }
    finally { setSaving(false); }
  };

  const sync = async () => {
    setSyncing(true); setSyncResult(null); setSyncErr('');
    try {
      await updateIntakeConfig({ github_owner: form.owner.trim(), github_repo: form.repo.trim(), github_token: form.token.trim(), github_project_id: projectId });
      const r = await syncGithubIssues();
      setSyncResult(r);
    } catch (e) { setSyncErr(String(e)); }
    finally { setSyncing(false); }
  };

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 14 }}>
      <ICard>
        <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginBottom: 18 }}>
          <div style={{ width: 34, height: 34, borderRadius: 9, background: 'rgba(79,142,209,.15)', display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
            <Icon name="code" size={16} style={{ color: 'var(--blue)' }} />
          </div>
          <div>
            <div style={{ fontWeight: 600, fontSize: 'var(--text-body)' }}>GitHub Issues 同步</div>
            <div style={{ fontSize: 'var(--text-label)', color: 'var(--text-3)', marginTop: 1 }}>单向拉取仓库 Issues，自动去重导入本项目</div>
          </div>
        </div>

        <ISectionLabel>仓库信息</ISectionLabel>
        <div className="cfg-fields" style={{ gridTemplateColumns: '1fr 1fr', gap: 10, marginBottom: 14 }}>
          <div className="field" style={{ margin: 0 }}>
            <label>Owner / Org</label>
            <input value={form.owner} onChange={e => setForm(f => ({ ...f, owner: e.target.value }))} placeholder="your-org" />
          </div>
          <div className="field" style={{ margin: 0 }}>
            <label>Repository</label>
            <input value={form.repo} onChange={e => setForm(f => ({ ...f, repo: e.target.value }))} placeholder="my-project" />
          </div>
        </div>

        <ISectionLabel>认证</ISectionLabel>
        <div className="field" style={{ margin: '0 0 14px' }}>
          <label>GitHub Token（私有仓库必填，公开仓库可留空）</label>
          <input type="password" value={form.token} onChange={e => setForm(f => ({ ...f, token: e.target.value }))}
            placeholder="ghp_xxxxxxxxxxxx" style={{ fontFamily: 'var(--font-mono)', fontSize: 'var(--text-label)' }} />
        </div>

        <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
          <button className="btn btn-primary" onClick={save} disabled={saving}>
            <Icon name="check" size={14} />{saving ? '保存中…' : '保存配置'}
          </button>
          {saveOk === true && <span style={{ fontSize: 'var(--text-label)', color: 'var(--green)' }}>✓ 已保存</span>}
          {saveOk === false && <span style={{ fontSize: 'var(--text-label)', color: 'var(--red)' }}>保存失败</span>}
        </div>
      </ICard>

      <ICard>
        <ISectionLabel>立即同步</ISectionLabel>
        {cfg.github_last_sync && (
          <div style={{ fontSize: 'var(--text-label)', color: 'var(--text-3)', marginBottom: 10 }}>
            上次同步：{new Date(cfg.github_last_sync).toLocaleString('zh-CN')}
          </div>
        )}
        {syncResult && (
          <IResultBanner ok>
            同步完成：导入 <strong>{syncResult.imported}</strong> 条，
            跳过重复 {syncResult.skipped} 条
            {syncResult.errors > 0 && `，错误 ${syncResult.errors} 条`}
          </IResultBanner>
        )}
        {syncErr && <IResultBanner ok={false}>{syncErr}</IResultBanner>}
        <button className="btn" style={{ marginTop: syncResult || syncErr ? 10 : 0 }}
          onClick={sync} disabled={syncing || !form.owner || !form.repo}>
          <Icon name="refresh" size={14} style={{ animation: syncing ? 'spin 1s linear infinite' : undefined }} />
          {syncing ? '同步中…' : '立即同步'}
        </button>
      </ICard>
    </div>
  );
}

// ── ProjectScannerTab ─────────────────────────────────────────────────────────

const SCAN_ITEMS_DEF = [
  { key: 'todo' as const,  icon: 'code',   color: '#8b7ad8', bg: 'rgba(139,122,216,.12)', title: 'TODO / FIXME 注释', desc: '扫描代码中的 TODO、FIXME、HACK、XXX 注释并作为 Debt 类需求入队' },
  { key: 'cargo' as const, icon: 'shield', color: '#db5a40', bg: 'rgba(219,90,64,.12)',   title: 'cargo audit',       desc: '检测 Rust 依赖中的已知安全漏洞（需 Cargo.lock）' },
  { key: 'npm' as const,   icon: 'box',    color: '#4f9d6b', bg: 'rgba(79,157,107,.12)',  title: 'npm audit',         desc: '检测 Node.js 依赖中的安全漏洞（需 package-lock.json）' },
];

function ProjectScannerTab({ projectId }: { projectId: string }) {
  const [scanTypes, setScanTypes] = useState({ todo: true, cargo: true, npm: true });
  const [scanning, setScanning]   = useState(false);
  const [result, setResult]       = useState<ScanResult | null>(null);
  const [scanErr, setScanErr]     = useState('');

  const scan = async () => {
    setScanning(true); setResult(null); setScanErr('');
    const types: string[] = [];
    if (scanTypes.todo) types.push('todo');
    if (scanTypes.cargo) types.push('cargo_audit');
    if (scanTypes.npm) types.push('npm_audit');
    try { setResult(await runCodeScan(projectId, types)); }
    catch (e) { setScanErr(String(e)); }
    finally { setScanning(false); }
  };

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 14 }}>
      <ICard>
        <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginBottom: 16 }}>
          <div style={{ width: 34, height: 34, borderRadius: 9, background: 'rgba(79,157,107,.15)', display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
            <Icon name="search" size={16} style={{ color: 'var(--green)' }} />
          </div>
          <div>
            <div style={{ fontWeight: 600, fontSize: 'var(--text-body)' }}>代码扫描</div>
            <div style={{ fontSize: 'var(--text-label)', color: 'var(--text-3)', marginTop: 1 }}>主动发现代码库中的问题并自动入队</div>
          </div>
        </div>

        <ISectionLabel>扫描类型</ISectionLabel>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 8, marginBottom: 14 }}>
          {SCAN_ITEMS_DEF.map(item => (
            <div key={item.key} onClick={() => setScanTypes(s => ({ ...s, [item.key]: !s[item.key] }))}
              style={{ display: 'flex', alignItems: 'center', gap: 12, padding: '12px 14px', borderRadius: 10, cursor: 'pointer', background: scanTypes[item.key] ? item.bg : 'var(--bg-3)', border: `1px solid ${scanTypes[item.key] ? item.color + '44' : 'var(--border)'}`, transition: 'background .15s, border-color .15s' }}>
              <div style={{ width: 30, height: 30, borderRadius: 8, background: item.bg, display: 'flex', alignItems: 'center', justifyContent: 'center', flexShrink: 0 }}>
                <Icon name={item.icon as any} size={14} style={{ color: item.color }} />
              </div>
              <div style={{ flex: 1, minWidth: 0 }}>
                <div style={{ fontWeight: 500, fontSize: 'var(--text-control)' }}>{item.title}</div>
                <div style={{ fontSize: 'var(--text-label)', color: 'var(--text-3)', marginTop: 2 }}>{item.desc}</div>
              </div>
              <div style={{ width: 18, height: 18, borderRadius: 5, flexShrink: 0, background: scanTypes[item.key] ? item.color : 'transparent', border: `2px solid ${scanTypes[item.key] ? item.color : 'var(--border-strong)'}`, display: 'flex', alignItems: 'center', justifyContent: 'center', transition: 'background .15s, border-color .15s' }}>
                {scanTypes[item.key] && <Icon name="check" size={10} style={{ color: '#fff' }} />}
              </div>
            </div>
          ))}
        </div>

        {result && <IResultBanner ok>扫描完成：发现 <strong>{result.found}</strong> 处，新建 Issue <strong>{result.new_issues}</strong> 条</IResultBanner>}
        {scanErr && <IResultBanner ok={false}>{scanErr}</IResultBanner>}
        <button className="btn btn-primary" style={{ marginTop: result || scanErr ? 10 : 0, alignSelf: 'flex-start' }}
          onClick={scan} disabled={scanning}>
          <Icon name="search" size={14} style={{ animation: scanning ? 'spin 1s linear infinite' : undefined }} />
          {scanning ? '扫描中…' : '运行扫描'}
        </button>
      </ICard>
    </div>
  );
}

// ── ProjectBulkTab ────────────────────────────────────────────────────────────

const BULK_FORMAT_META = {
  text: { label: '纯文本', desc: '每行一个需求标题' },
  csv:  { label: 'CSV',   desc: '支持 title / description / category / severity 列' },
  json: { label: 'JSON',  desc: '对象数组，字段同 CSV' },
} as const;

const BULK_PLACEHOLDERS: Record<string, string> = {
  text: '每行一个需求标题，例如：\n添加用户头像上传功能\n优化搜索响应速度\n修复登录超时 Bug',
  csv:  'title,description,category,severity\n添加头像上传,用户可上传个人头像,Feature,medium\n修复登录 Bug,,Bug,high',
  json: '[\n  { "title": "需求1", "description": "描述", "category": "Feature", "severity": "medium" },\n  { "title": "需求2" }\n]',
};

function ProjectBulkTab({ projectId }: { projectId: string }) {
  const [format, setFormat]   = useState<'text' | 'csv' | 'json'>('text');
  const [content, setContent] = useState('');
  const [importing, setImporting] = useState(false);
  const [result, setResult]   = useState<BulkResult | null>(null);
  const [importErr, setImportErr] = useState('');

  const doImport = async () => {
    if (!content.trim()) return;
    setImporting(true); setResult(null); setImportErr('');
    try { setResult(await bulkImportIssues(projectId, format, content)); }
    catch (e) { setImportErr(String(e)); }
    finally { setImporting(false); }
  };

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 14 }}>
      <ICard>
        <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginBottom: 18 }}>
          <div style={{ width: 34, height: 34, borderRadius: 9, background: 'rgba(224,163,46,.15)', display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
            <Icon name="arrowUp" size={16} style={{ color: 'var(--amber)' }} />
          </div>
          <div>
            <div style={{ fontWeight: 600, fontSize: 'var(--text-body)' }}>批量导入</div>
            <div style={{ fontSize: 'var(--text-label)', color: 'var(--text-3)', marginTop: 1 }}>粘贴多条需求，一次性入队分析（最多 200 条）</div>
          </div>
        </div>

        <ISectionLabel>格式</ISectionLabel>
        <div style={{ display: 'flex', gap: 6, marginBottom: 14 }}>
          {(Object.entries(BULK_FORMAT_META) as [keyof typeof BULK_FORMAT_META, { label: string; desc: string }][]).map(([key, meta]) => (
            <button key={key} className={'btn btn-sm' + (format === key ? ' btn-primary' : '')}
              onClick={() => { setFormat(key); setContent(''); }}
              style={{ flexDirection: 'column', alignItems: 'flex-start', padding: '8px 12px', height: 'auto', gap: 2 }}>
              <span style={{ fontWeight: 600, fontSize: 'var(--text-label)' }}>{meta.label}</span>
              <span style={{ fontSize: 'var(--text-caption)', color: format === key ? 'rgba(255,255,255,.75)' : 'var(--text-2)', fontWeight: 400 }}>{meta.desc}</span>
            </button>
          ))}
        </div>
      </ICard>

      <ICard>
        <ISectionLabel>内容（{content.split('\n').filter(l => l.trim()).length} 行）</ISectionLabel>
        <textarea value={content} onChange={e => setContent(e.target.value)}
          placeholder={BULK_PLACEHOLDERS[format]} rows={10}
          style={{ width: '100%', boxSizing: 'border-box', fontFamily: 'var(--font-mono)', fontSize: 'var(--text-label)', lineHeight: 'var(--leading-relaxed)', background: 'var(--bg-3)', border: '1px solid var(--border)', borderRadius: 8, padding: '10px 12px', color: 'var(--text)', resize: 'vertical', outline: 'none' }} />
      </ICard>

      {result && (
        <IResultBanner ok={result.errors.length === 0}>
          <div>导入完成：共 {result.total} 条，成功 <strong>{result.imported}</strong>，跳过 {result.skipped}</div>
          {result.errors.length > 0 && (
            <div style={{ marginTop: 6 }}>
              {result.errors.slice(0, 5).map((e, i) => <div key={i} style={{ marginTop: 2 }}>• {e}</div>)}
              {result.errors.length > 5 && <div>…及 {result.errors.length - 5} 条其他错误</div>}
            </div>
          )}
        </IResultBanner>
      )}
      {importErr && <IResultBanner ok={false}>{importErr}</IResultBanner>}

      <button className="btn btn-primary" style={{ alignSelf: 'flex-start' }}
        onClick={doImport} disabled={importing || !content.trim()}>
        <Icon name="arrowUp" size={14} />{importing ? '导入中…' : '开始导入'}
      </button>
    </div>
  );
}

// ── SpecPanel ─────────────────────────────────────────────────────────────────

const SPEC_CATEGORIES: { id: SpecCategory; label: string; icon: string; hint: string }[] = [
  { id: 'tech_stack',   label: '技术栈',   icon: 'cpu',     hint: '语言版本、框架、禁止使用的库' },
  { id: 'architecture', label: '架构约束', icon: 'layers',  hint: '分层规则、模块边界、命名约定' },
  { id: 'coding',       label: '编码规范', icon: 'code',    hint: '错误处理、日志格式、注释语言' },
  { id: 'api',          label: 'API 契约', icon: 'zap',     hint: '接口风格、鉴权方式、版本策略' },
  { id: 'testing',      label: '测试要求', icon: 'check',   hint: '覆盖率门槛、必测模块' },
];

function SpecCard({ spec, onEdit, onDelete }: {
  spec: ProjectSpec;
  onEdit: (s: ProjectSpec) => void;
  onDelete: (s: ProjectSpec) => void;
}) {
  const [hovered, setHovered] = useState(false);
  return (
    <div
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
      style={{ background: 'var(--bg-3)', border: '1px solid var(--border)', borderRadius: 10, padding: '11px 14px', position: 'relative' }}
    >
      <div style={{ display: 'flex', alignItems: 'flex-start', gap: 8 }}>
        <div style={{ flex: 1, minWidth: 0 }}>
          <div style={{ fontSize: 'var(--text-control)', fontWeight: 600, color: 'var(--text-1)', marginBottom: 4 }}>{spec.title}</div>
          {spec.content && (
            <div style={{ fontSize: 'var(--text-label)', color: 'var(--text-3)', lineHeight: 'var(--leading-relaxed)', whiteSpace: 'pre-wrap', wordBreak: 'break-word' }}>{spec.content}</div>
          )}
        </div>
        {hovered && (
          <div style={{ display: 'flex', gap: 3, flexShrink: 0 }}>
            <button className="btn btn-sm" style={{ padding: '2px 6px' }} onClick={() => onEdit(spec)} title="编辑">
              <Icon name="edit" size={11} />
            </button>
            <button className="btn btn-sm" style={{ padding: '2px 6px', color: 'var(--red)' }} onClick={() => onDelete(spec)} title="删除">
              <Icon name="trash" size={11} />
            </button>
          </div>
        )}
      </div>
    </div>
  );
}

function SpecEditModal({ spec, category, projectId, onSave, onClose }: {
  spec: ProjectSpec | null;
  category: SpecCategory;
  projectId: string;
  onSave: (s: ProjectSpec) => void;
  onClose: () => void;
}) {
  const [title, setTitle]     = useState(spec?.title ?? '');
  const [content, setContent] = useState(spec?.content ?? '');
  const [saving, setSaving]   = useState(false);
  const [err, setErr]         = useState('');

  const save = async () => {
    setSaving(true); setErr('');
    try {
      const result = await upsertProjectSpec(projectId, spec?.id ?? null, category, title, content);
      onSave(result);
    } catch (e) { setErr(String(e)); }
    finally { setSaving(false); }
  };

  const catMeta = SPEC_CATEGORIES.find(c => c.id === category)!;

  return (
    <div style={{ position: 'fixed', inset: 0, background: 'rgba(0,0,0,.5)', backdropFilter: 'blur(3px)', display: 'grid', placeItems: 'center', zIndex: 220 }} onClick={onClose}>
      <div style={{ width: 520, background: 'var(--bg-2)', border: '1px solid var(--border-strong)', borderRadius: 18, boxShadow: 'var(--shadow-lg)', padding: '22px 24px' }} onClick={e => e.stopPropagation()}>
        <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 18 }}>
          <div className="eyebrow" style={{ fontSize: 'var(--text-control)' }}>
            <span className="en">{catMeta.label.toUpperCase()}</span>
            <span className="cn"> · {spec ? '编辑规格' : '新增规格'}</span>
          </div>
          <button className="icon-btn" onClick={onClose}><Icon name="x" size={16} /></button>
        </div>
        {err && <div style={{ color: 'var(--red)', fontSize: 'var(--text-label)', marginBottom: 12, padding: '6px 10px', background: 'rgba(219,90,64,.08)', borderRadius: 6 }}>{err}</div>}
        <div className="field" style={{ marginBottom: 14 }}>
          <label>标题</label>
          <input autoFocus placeholder="简短的规格名称" value={title} onChange={e => setTitle(e.target.value)}
            onKeyDown={e => { if (e.key === 'Enter' && e.ctrlKey) save(); if (e.key === 'Escape') onClose(); }} />
        </div>
        <div className="field" style={{ marginBottom: 20 }}>
          <label>内容 <span style={{ fontWeight: 400, color: 'var(--text-faint)', fontSize: 'var(--text-caption)' }}>（{catMeta.hint}）</span></label>
          <textarea rows={5} placeholder="具体约束说明…" value={content} onChange={e => setContent(e.target.value)}
            style={{ resize: 'vertical', fontFamily: 'var(--font-mono)', fontSize: 'var(--text-label)' }} />
        </div>
        <div style={{ display: 'flex', justifyContent: 'flex-end', gap: 8 }}>
          <button className="btn btn-sm" onClick={onClose}>取消</button>
          <button className="btn btn-sm btn-primary" onClick={save} disabled={saving || !title.trim()}>
            {saving ? '保存中…' : '保存'}
          </button>
        </div>
      </div>
    </div>
  );
}

function SpecPanel({ projectId }: { projectId: string }) {
  const [specs, setSpecs]           = useState<ProjectSpec[]>([]);
  const [activeCategory, setActive] = useState<SpecCategory>('tech_stack');
  const [loading, setLoading]       = useState(true);
  const [aiWorking, setAiWorking]   = useState(false);
  const [message, setMessage]       = useState('');
  const [error, setError]           = useState('');
  const [editing, setEditing]       = useState<ProjectSpec | null | undefined>(undefined);
  // undefined = modal closed, null = new item, ProjectSpec = editing existing

  const load = useCallback(async () => {
    try {
      setSpecs(await listProjectSpecs(projectId));
    } catch (e) { setError(String(e)); }
    finally { setLoading(false); }
  }, [projectId]);

  useEffect(() => { setLoading(true); load(); }, [load]);

  const flash = (msg: string) => { setMessage(msg); setTimeout(() => setMessage(''), 4000); };

  const doAiGenerate = async () => {
    if (!confirm('AI 将分析项目信息并重新生成所有分类规格，现有规格将被覆盖。确认继续？')) return;
    setAiWorking(true); setError('');
    try { flash(await aiGenerateSpecs(projectId)); await load(); }
    catch (e) { setError(String(e)); }
    finally { setAiWorking(false); }
  };

  const doDelete = async (s: ProjectSpec) => {
    if (!confirm(`确认删除规格「${s.title}」？`)) return;
    try { await deleteProjectSpec(s.id); await load(); }
    catch (e) { setError(String(e)); }
  };

  const onSaved = async () => { setEditing(undefined); await load(); };

  const categorySpecs = specs.filter(s => s.category === activeCategory);
  const countByCategory = (id: SpecCategory) => specs.filter(s => s.category === id).length;

  return (
    <div style={{ flex: 1, display: 'flex', flexDirection: 'column', overflow: 'hidden', minHeight: 0 }}>

      {/* action bar */}
      <div style={{ display: 'flex', alignItems: 'center', gap: 6, padding: '10px 20px', borderBottom: '1px solid var(--border)', flexShrink: 0, flexWrap: 'wrap' }}>
        <button className="btn btn-sm btn-primary" onClick={() => setEditing(null)}>
          <Icon name="plus" size={13} />新增规格
        </button>
        <div style={{ flex: 1 }} />
        {message && <span style={{ fontSize: 'var(--text-label)', color: 'var(--text-3)', maxWidth: 300, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{message}</span>}
        <button className="btn btn-sm" onClick={doAiGenerate} disabled={aiWorking}>
          <Icon name="brain" size={13} style={aiWorking ? { animation: 'spin 1s linear infinite' } : undefined} />
          {aiWorking ? 'AI 生成中…' : 'AI 一键生成'}
        </button>
      </div>

      {error && (
        <div style={{ padding: '6px 20px', color: 'var(--red)', fontSize: 'var(--text-label)', background: 'rgba(219,90,64,.06)', flexShrink: 0 }}>{error}</div>
      )}

      {/* body: category sidebar + spec list */}
      <div style={{ flex: 1, display: 'flex', overflow: 'hidden', minHeight: 0 }}>

        {/* category sidebar */}
        <div style={{ width: 160, flexShrink: 0, borderRight: '1px solid var(--border)', padding: '12px 8px', display: 'flex', flexDirection: 'column', gap: 2 }}>
          {SPEC_CATEGORIES.map(cat => {
            const count = countByCategory(cat.id);
            const active = activeCategory === cat.id;
            return (
              <button key={cat.id} onClick={() => setActive(cat.id)}
                className="btn btn-sm"
                style={{ justifyContent: 'flex-start', gap: 7, background: active ? 'var(--ember-tint)' : 'transparent', color: active ? 'var(--ember)' : 'var(--text-2)', border: 'none', padding: '6px 10px' }}
              >
                <Icon name={cat.icon as any} size={13} style={{ flexShrink: 0 }} />
                <span style={{ flex: 1, textAlign: 'left' }}>{cat.label}</span>
                {count > 0 && <span style={{ fontSize: 'var(--text-micro)', color: active ? 'var(--ember)' : 'var(--text-faint)', background: active ? 'rgba(232,119,46,.15)' : 'var(--bg-3)', borderRadius: 8, padding: '1px 6px' }}>{count}</span>}
              </button>
            );
          })}
          <div style={{ flex: 1 }} />
          <div style={{ fontSize: 'var(--text-caption)', color: 'var(--text-faint)', lineHeight: 'var(--leading-normal)', padding: '8px 6px' }}>
            规格文件存储于项目目录<br />
            <code style={{ fontSize: 'var(--text-micro)' }}>.autoforge/specs/</code>
          </div>
        </div>

        {/* spec list */}
        <div style={{ flex: 1, overflowY: 'auto', padding: '16px 20px' }}>
          {loading ? (
            <div style={{ color: 'var(--text-3)', fontSize: 'var(--text-control)' }}>加载中…</div>
          ) : categorySpecs.length === 0 ? (
            <div className="empty" style={{ minHeight: 180 }}>
              <Icon name={SPEC_CATEGORIES.find(c => c.id === activeCategory)!.icon as any} size={32} style={{ opacity: .2 }} />
              <div>暂无{SPEC_CATEGORIES.find(c => c.id === activeCategory)!.label}规格</div>
              <div style={{ fontSize: 'var(--text-label)', color: 'var(--text-faint)' }}>{SPEC_CATEGORIES.find(c => c.id === activeCategory)!.hint}</div>
              <button className="btn btn-sm" onClick={() => setEditing(null)}><Icon name="plus" size={12} />新增规格</button>
            </div>
          ) : (
            <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
              {categorySpecs.map(s => (
                <SpecCard key={s.id} spec={s} onEdit={s => setEditing(s)} onDelete={doDelete} />
              ))}
              <button className="btn btn-sm" style={{ alignSelf: 'flex-start', marginTop: 4 }} onClick={() => setEditing(null)}>
                <Icon name="plus" size={12} />添加规格
              </button>
            </div>
          )}
        </div>
      </div>

      {editing !== undefined && (
        <SpecEditModal
          spec={editing}
          category={activeCategory}
          projectId={projectId}
          onSave={onSaved}
          onClose={() => setEditing(undefined)}
        />
      )}
    </div>
  );
}

// ── IntakePanel ───────────────────────────────────────────────────────────────

type IntakeSubTab = 'github' | 'scanner' | 'bulk';

const INTAKE_SUB_TABS: { id: IntakeSubTab; label: string; ic: string }[] = [
  { id: 'github',  label: 'GitHub',   ic: 'code' },
  { id: 'scanner', label: '代码扫描', ic: 'search' },
  { id: 'bulk',    label: '批量导入', ic: 'arrowUp' },
];

function IntakePanel({ projectId }: { projectId: string }) {
  const [subTab, setSubTab] = useState<IntakeSubTab>('github');
  const [cfg, setCfg]       = useState<IntakeConfig | null>(null);
  const [loading, setLoading] = useState(true);
  const [loadErr, setLoadErr] = useState('');

  useEffect(() => {
    setLoading(true);
    getIntakeConfig()
      .then(setCfg)
      .catch(e => setLoadErr(String(e)))
      .finally(() => setLoading(false));
  }, [projectId]);

  return (
    <div style={{ flex: 1, display: 'flex', flexDirection: 'column', overflow: 'hidden', minHeight: 0 }}>
      {/* sub-tab bar */}
      <div style={{ display: 'flex', gap: 2, padding: '10px 24px 0', borderBottom: '1px solid var(--border)', flexShrink: 0 }}>
        {INTAKE_SUB_TABS.map(t => (
          <button key={t.id} onClick={() => setSubTab(t.id)}
            style={{ background: 'none', border: 'none', padding: '6px 14px', cursor: 'pointer', fontSize: 'var(--text-control)', fontWeight: 600, display: 'flex', alignItems: 'center', gap: 6, color: subTab === t.id ? 'var(--ember)' : 'var(--text-3)', borderBottom: subTab === t.id ? '2px solid var(--ember)' : '2px solid transparent', marginBottom: -1, transition: 'color .15s' }}>
            <Icon name={t.ic as any} size={13} />{t.label}
          </button>
        ))}
      </div>

      {/* content */}
      <div style={{ flex: 1, overflowY: 'auto', padding: '20px 24px' }}>
        {loadErr && <IResultBanner ok={false}>{loadErr}</IResultBanner>}
        {loading ? (
          <div style={{ color: 'var(--text-3)', fontSize: 'var(--text-control)' }}>加载中…</div>
        ) : cfg ? (
          <>
            {subTab === 'github'  && <ProjectGithubTab projectId={projectId} cfg={cfg} onCfgChange={setCfg} />}
            {subTab === 'scanner' && <ProjectScannerTab projectId={projectId} />}
            {subTab === 'bulk'    && <ProjectBulkTab projectId={projectId} />}
          </>
        ) : null}
      </div>
    </div>
  );
}

// ── ProjectInfoTab ────────────────────────────────────────────────────────────

function ProjectInfoTab({ project }: { project: Project }) {
  const rows: { label: string; value: React.ReactNode }[] = [
    { label: '仓库路径', value: <code style={{ fontFamily: 'var(--font-mono)', fontSize: 'var(--text-label)' }}>{project.repo_path || '未配置'}</code> },
    { label: '开发分支', value: <code style={{ fontFamily: 'var(--font-mono)', fontSize: 'var(--text-label)' }}>{project.branch_dev}</code> },
    { label: '主分支',   value: <code style={{ fontFamily: 'var(--font-mono)', fontSize: 'var(--text-label)' }}>{project.branch_main}</code> },
    { label: '项目标识', value: <code style={{ fontFamily: 'var(--font-mono)', fontSize: 'var(--text-label)' }}>{project.slug}</code> },
    { label: '创建时间', value: <span style={{ fontSize: 'var(--text-label)', color: 'var(--text-3)' }}>{project.created_at.replace('T', ' ').replace('Z', '')}</span> },
  ];

  return (
    <div style={{ padding: '20px 24px', overflowY: 'auto', height: '100%' }}>
      {project.description && (
        <div style={{ fontSize: 'var(--text-control)', color: 'var(--text-2)', marginBottom: 20, lineHeight: 'var(--leading-relaxed)', padding: '12px 14px', background: 'var(--bg-3)', borderRadius: 10, border: '1px solid var(--border)' }}>
          {project.description}
        </div>
      )}
      <div style={{ display: 'flex', flexDirection: 'column', gap: 0 }}>
        {rows.map(r => (
          <div key={r.label} style={{ display: 'flex', alignItems: 'center', padding: '10px 0', borderBottom: '1px solid var(--border)' }}>
            <div style={{ width: 90, fontSize: 'var(--text-label)', color: 'var(--text-faint)', flexShrink: 0 }}>{r.label}</div>
            <div style={{ flex: 1 }}>{r.value}</div>
          </div>
        ))}
      </div>

      {project.config_yaml && (
        <details style={{ marginTop: 20 }}>
          <summary style={{ fontSize: 'var(--text-label)', color: 'var(--text-3)', cursor: 'pointer', userSelect: 'none' }}>项目配置 YAML</summary>
          <pre style={{ marginTop: 8, padding: '10px 12px', background: 'var(--bg-3)', borderRadius: 8, fontSize: 'var(--text-label)', fontFamily: 'var(--font-mono)', overflowX: 'auto', color: 'var(--text-2)', border: '1px solid var(--border)' }}>
            {project.config_yaml}
          </pre>
        </details>
      )}
    </div>
  );
}

// ── ProjectNavItem ────────────────────────────────────────────────────────────

function ProjectNavItem({ project, active, onClick }: {
  project: Project; active: boolean; onClick: () => void;
}) {
  return (
    <div
      className={'set-nav-item' + (active ? ' active' : '')}
      onClick={onClick}
      style={{ flexDirection: 'column', alignItems: 'flex-start', gap: 3, padding: '9px 12px' }}
    >
      <div style={{ display: 'flex', alignItems: 'center', gap: 8, width: '100%' }}>
        <div style={{ width: 26, height: 26, borderRadius: 7, background: active ? 'var(--ember)' : 'var(--bg-3)', display: 'flex', alignItems: 'center', justifyContent: 'center', fontSize: 'var(--text-label)', fontWeight: 800, color: active ? '#fff' : 'var(--text-3)', flexShrink: 0, fontFamily: 'var(--font-display)' }}>
          {project.name[0]}
        </div>
        <span style={{ flex: 1, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', fontSize: 'var(--text-body)', fontWeight: 600 }}>
          {project.name}
        </span>
        <span className={'chip ' + (project.status === 'active' ? 'green' : '')} style={{ fontSize: 'var(--text-micro)', padding: '1px 5px', flexShrink: 0 }}>
          {project.status === 'active' ? '启用' : '停用'}
        </span>
      </div>
      <div style={{ paddingLeft: 34, fontSize: 'var(--text-caption)', color: active ? 'var(--ember-soft)' : 'var(--text-faint)', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', width: '100%' }}>
        {project.description || project.slug}
      </div>
    </div>
  );
}

// ── ProjectsPage ──────────────────────────────────────────────────────────────

type Tab = 'info' | 'materials' | 'intake' | 'spec';

export default function ProjectsPage() {
  const [projects, setProjects]         = useState<Project[]>([]);
  const [loading, setLoading]           = useState(true);
  const [error, setError]               = useState('');
  const [selectedId, setSelectedId]     = useState<string | null>(null);
  const [activeTab, setActiveTab]       = useState<Tab>('info');
  const [showCreate, setShowCreate]     = useState(false);
  const [editProject, setEditProject]   = useState<Project | null>(null);
  const [deleteTarget, setDeleteTarget] = useState<Project | null>(null);
  const [deleting, setDeleting]         = useState(false);

  const selectedProject = projects.find(p => p.id === selectedId) ?? null;

  const load = useCallback(async () => {
    setError('');
    try {
      const ps = await listProjects();
      setProjects(ps);
      // auto-select first project if none selected
      setSelectedId(id => {
        if (id && ps.some(p => p.id === id)) return id;
        return ps[0]?.id ?? null;
      });
    } catch (e) { setError(String(e)); }
    finally { setLoading(false); }
  }, []);

  useEffect(() => {
    load();
    let unlisten: (() => void) | undefined;
    listen('AutoForge://event', () => load()).then(fn => { unlisten = fn; });
    return () => unlisten?.();
  }, [load]);

  const doDelete = async () => {
    if (!deleteTarget) return;
    setDeleting(true);
    try {
      await deleteProject(deleteTarget.id);
      setDeleteTarget(null);
      if (selectedId === deleteTarget.id) setSelectedId(null);
      await load();
      window.dispatchEvent(new Event('AutoForge:badges-refresh'));
    } catch (e) { setError(String(e)); setDeleteTarget(null); }
    finally { setDeleting(false); }
  };

  const doToggleStatus = async (project: Project) => {
    const nextStatus = project.status === 'active' ? 'inactive' : 'active';
    setError('');
    try {
      const saved = await updateProject(project.id, { status: nextStatus });
      setProjects(ps => ps.map(p => p.id === saved.id ? saved : p));
      window.dispatchEvent(new Event('AutoForge:badges-refresh'));
    } catch (e) {
      setError(String(e));
    }
  };

  const selectProject = (id: string) => {
    setSelectedId(id);
    setActiveTab('info');
  };

  return (
    <div className="content">
      {/* top bar */}
      <div className="audit-top" style={{ height: 56 }}>
        <div className="eyebrow" style={{ fontSize: 'var(--text-heading)' }}>
          <span className="en">PROJECTS</span><span className="cn">· 项目管理</span>
        </div>
        <button className="btn btn-primary btn-sm" onClick={() => setShowCreate(true)}>
          <Icon name="plus" size={14} />新建项目
        </button>
      </div>

      {/* left-right split */}
      <div className="set-wrap">

        {/* left: project list */}
        <div className="set-nav" style={{ width: 220, overflowY: 'auto', display: 'flex', flexDirection: 'column' }}>
          {error && (
            <div style={{ fontSize: 'var(--text-caption)', color: 'var(--red)', padding: '6px 10px', margin: '0 0 6px' }}>{error}</div>
          )}
          {loading ? (
            <div style={{ fontSize: 'var(--text-label)', color: 'var(--text-faint)', padding: '12px 12px' }}>加载中…</div>
          ) : projects.length === 0 ? (
            <div style={{ padding: '24px 12px', textAlign: 'center' }}>
              <div className="empty-line" style={{ marginBottom: 10 }}>暂无项目</div>
              <button className="btn btn-sm btn-primary" onClick={() => setShowCreate(true)}>
                <Icon name="plus" size={12} />新建项目
              </button>
            </div>
          ) : (
            projects.map(p => (
              <ProjectNavItem key={p.id} project={p} active={selectedId === p.id} onClick={() => selectProject(p.id)} />
            ))
          )}
        </div>

        {/* right: project detail */}
        <div style={{ flex: 1, display: 'flex', flexDirection: 'column', overflow: 'hidden', minWidth: 0 }}>
          {selectedProject ? (
            <>
              {/* project header */}
              <div style={{ padding: '14px 24px 0', flexShrink: 0 }}>
                <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginBottom: 14 }}>
                  <div style={{ width: 44, height: 44, borderRadius: 12, background: 'var(--ember)', display: 'flex', alignItems: 'center', justifyContent: 'center', fontSize: 'var(--text-heading)', fontWeight: 800, color: '#fff', fontFamily: 'var(--font-display)', flexShrink: 0 }}>
                    {selectedProject.name[0]}
                  </div>
                  <div style={{ flex: 1, minWidth: 0 }}>
                    <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                      <span style={{ fontSize: 'var(--text-section)', fontWeight: 700 }}>{selectedProject.name}</span>
                      <span className={'chip ' + (selectedProject.status === 'active' ? 'green' : '')} style={{ fontSize: 'var(--text-micro)', padding: '1px 7px' }}>
                        {selectedProject.status === 'active' ? '启用中' : '已停用'}
                      </span>
                    </div>
                    <div style={{ fontSize: 'var(--text-label)', color: 'var(--text-faint)', fontFamily: 'var(--font-mono)', marginTop: 2 }}>
                      {selectedProject.repo_path || '仓库路径未配置'}
                    </div>
                  </div>
                  <div style={{ display: 'flex', gap: 6, flexShrink: 0 }}>
                    <button className="btn btn-sm" onClick={() => doToggleStatus(selectedProject)}>
                      <Icon name={selectedProject.status === 'active' ? 'pause' : 'play'} size={13} />
                      {selectedProject.status === 'active' ? '停用' : '启用'}
                    </button>
                    <button className="btn btn-sm" onClick={() => setEditProject(selectedProject)}>
                      <Icon name="edit" size={13} />编辑
                    </button>
                    <button className="btn btn-sm" style={{ color: 'var(--red)' }} onClick={() => setDeleteTarget(selectedProject)}>
                      <Icon name="trash" size={13} />删除
                    </button>
                  </div>
                </div>

                {/* tabs */}
                <div style={{ display: 'flex', gap: 2, borderBottom: '1px solid var(--border)' }}>
                  {([['info', '基本信息', 'sliders'], ['materials', '物料库', 'folder'], ['intake', '需求入口', 'inbox'], ['spec', '规格', 'layers']] as const).map(([id, label, ic]) => (
                    <button
                      key={id}
                      onClick={() => setActiveTab(id as Tab)}
                      style={{
                        background: 'none', border: 'none', padding: '7px 14px', cursor: 'pointer',
                        fontSize: 'var(--text-control)', fontWeight: 600, display: 'flex', alignItems: 'center', gap: 6,
                        color: activeTab === id ? 'var(--ember)' : 'var(--text-3)',
                        borderBottom: activeTab === id ? '2px solid var(--ember)' : '2px solid transparent',
                        marginBottom: -1, transition: 'color .15s',
                      }}
                    >
                      <Icon name={ic} size={14} />{label}
                    </button>
                  ))}
                </div>
              </div>

              {/* tab content */}
              <div style={{ flex: 1, overflow: 'hidden', minHeight: 0, display: 'flex', flexDirection: 'column' }}>
                {activeTab === 'info'      && <ProjectInfoTab project={selectedProject} />}
                {activeTab === 'materials' && <MaterialsPanel key={selectedProject.id} projectId={selectedProject.id} />}
                {activeTab === 'intake'    && <IntakePanel key={selectedProject.id} projectId={selectedProject.id} />}
                {activeTab === 'spec'      && <SpecPanel key={selectedProject.id} projectId={selectedProject.id} />}
              </div>
            </>
          ) : (
            <div className="empty" style={{ height: '100%' }}>
              <Icon name="box" size={40} style={{ opacity: .3 }} />
              <div>
                {projects.length === 0 && !loading ? '还没有项目' : '从左侧选择一个项目'}
              </div>
            </div>
          )}
        </div>
      </div>

      {/* modals */}
      {showCreate && (
        <ProjectCreateModal
          onClose={() => setShowCreate(false)}
          onCreated={async p => { setShowCreate(false); await load(); setSelectedId(p.id); }}
        />
      )}
      {editProject && (
        <ProjectEditModal
          project={editProject}
          onClose={() => setEditProject(null)}
          onSaved={async () => { setEditProject(null); await load(); }}
        />
      )}
      {deleteTarget && (
        <ConfirmProjectDeleteModal
          project={deleteTarget}
          onCancel={() => setDeleteTarget(null)}
          onConfirm={doDelete}
        />
      )}
    </div>
  );
}
