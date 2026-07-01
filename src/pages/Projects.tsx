import React, { useState, useEffect, useCallback, useRef } from 'react';
import { listen } from '@tauri-apps/api/event';
import Icon from '../components/Icon';
import Select from '../components/Select';
import { ProjectCreateModal, ProjectEditModal, ConfirmProjectDeleteModal, ConfirmModal } from '../components/ProjectDialogs';
import IntakePanel from '../components/IntakePanel';
import { parseTs, fmtFull } from '../utils/datetime';
import {
  listProjects, updateProject, deleteProject, setDefaultProject, type Project,
  listArchivedProjects, restoreProject, purgeProject,
  listCodeAgents, setProjectCodeAgent, type CodeAgent as CodeAgentT,
  listMaterialFolders, createMaterialFolder, renameMaterialFolder, deleteMaterialFolder,
  listMaterialFiles, searchMaterialFiles, importMaterialFile, moveMaterialFile, deleteMaterialFile,
  openMaterialFile, aiOrganizeMaterials, backupMaterialFiles,
  getMaterialBackupConfig, updateMaterialBackupConfig,
  listProjectSpecs, upsertProjectSpec, deleteProjectSpec, aiGenerateSpecs,
  scanSpecFiles, getSpecContent, setSpecInjection,
  aiGenerateRunConfig, detectProjectCategory,
  type MaterialFolder, type MaterialFile, type MaterialBackupConfig,
  type MaterialSearchResult,
  type ProjectSpec, type SpecCategory, type SpecInjection, type RunConfigDraft,
} from '../services';
import {
  parseProjectConfigForm, buildProjectConfig,
  type ProjectConfigForm, type SensitiveFieldRow, type MaskRule,
} from '../utils/projectConfig';

// ── helpers ───────────────────────────────────────────────────────────────────

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

function formatMaterialTime(value: string): string {
  const d = parseTs(value);
  if (!d) return '—';
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

const RESERVED_ENTRY_NAMES = new Set([
  'CON', 'PRN', 'AUX', 'NUL',
  'COM1', 'COM2', 'COM3', 'COM4', 'COM5', 'COM6', 'COM7', 'COM8', 'COM9',
  'LPT1', 'LPT2', 'LPT3', 'LPT4', 'LPT5', 'LPT6', 'LPT7', 'LPT8', 'LPT9',
]);

function validateProjectEntryName(name: string, label: string): string | null {
  if (!name.trim()) return `${label}不能为空`;
  if (name !== name.trim()) return `${label}首尾不能包含空格`;
  if (name === '.' || name === '..') return `${label}不能为 . 或 ..`;
  if (/[<>:"/\\|?*\x00-\x1F]/.test(name)) return `${label}不能包含 < > : " / \\ | ? * 或控制字符`;
  if (/[ .]$/.test(name)) return `${label}不能以空格或句点结尾`;
  if (name.length > 255) return `${label}不能超过 255 个字符`;

  const baseName = name.split('.')[0]?.toUpperCase() ?? '';
  if (RESERVED_ENTRY_NAMES.has(baseName)) return `${label}不能使用 Windows 保留名称 ${baseName}`;
  return null;
}

function buildTree(folders: MaterialFolder[], parentId: string | null): MaterialFolder[] {
  return folders
    .filter(f => f.parent_id === parentId)
    .sort((a, b) => a.sort_order - b.sort_order || a.name.localeCompare(b.name));
}

function visibleMaterialFolders(folders: MaterialFolder[]): MaterialFolder[] {
  return folders;
}

function countFilesInFolderTree(folderId: string, folders: MaterialFolder[], files: MaterialFile[]): number {
  const childIds = folders.filter(f => f.parent_id === folderId).map(f => f.id);
  return files.filter(f => f.folder_id === folderId).length
    + childIds.reduce((sum, id) => sum + countFilesInFolderTree(id, folders, files), 0);
}

function folderTreeIds(folderId: string, folders: MaterialFolder[]): Set<string> {
  const ids = new Set<string>([folderId]);
  const pending = [folderId];
  while (pending.length > 0) {
    const parentId = pending.pop()!;
    for (const folder of folders) {
      if (folder.parent_id === parentId && !ids.has(folder.id)) {
        ids.add(folder.id);
        pending.push(folder.id);
      }
    }
  }
  return ids;
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
    <div style={{ position: 'fixed', inset: 'var(--win-gutter,0)', borderRadius: 14, background: 'rgba(0,0,0,.5)', backdropFilter: 'blur(3px)', display: 'grid', placeItems: 'center', zIndex: 220 }}>
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

function FileCard({ file, searchMeta, onOpen, onDelete, onMove }: {
  file: MaterialFile;
  searchMeta?: { folderPath: string; reason: string; preview: string | null };
  onOpen: () => void; onDelete: () => void; onMove: () => void;
}) {
  const [hovered, setHovered] = useState(false);
  const updatedTitle = fmtFull(file.updated_at);
  const backupChip = file.backup_status === 'synced'
    ? <span className="chip green" style={{ fontSize: 'var(--text-micro)', padding: '1px 5px' }}>已备份</span>
    : file.backup_status === 'error'
    ? <span className="chip red" style={{ fontSize: 'var(--text-micro)', padding: '1px 5px' }}>备份失败</span>
    : null;

  return (
    <div
      style={{
        background: 'var(--bg-2)', border: `1px solid ${hovered ? 'var(--border-strong)' : 'var(--border)'}`,
        borderRadius: 10, padding: '9px 10px 9px 12px', minHeight: searchMeta ? 82 : 54,
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
        {searchMeta && (
          <div style={{ display: 'flex', alignItems: 'center', gap: 7, marginTop: 6, minWidth: 0 }}>
            <span style={{ fontSize: 'var(--text-micro)', color: 'var(--ember)', background: 'var(--ember-tint)', borderRadius: 5, padding: '2px 6px', flexShrink: 0 }}>{searchMeta.reason}</span>
            <span title={searchMeta.folderPath} style={{ fontSize: 'var(--text-caption)', color: 'var(--text-faint)', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
              {searchMeta.folderPath}
            </span>
          </div>
        )}
        {searchMeta?.preview && (
          <div style={{ fontSize: 'var(--text-caption)', color: 'var(--text-3)', marginTop: 5, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', maxWidth: 680 }}>
            {searchMeta.preview.replace(/\s+/g, ' ').trim()}
          </div>
        )}
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
  return (
    <div style={{ position: 'fixed', inset: 'var(--win-gutter,0)', borderRadius: 14, background: 'rgba(0,0,0,.5)', backdropFilter: 'blur(3px)', display: 'grid', placeItems: 'center', zIndex: 220 }}>
      <div style={{ width: 360, background: 'var(--bg-2)', border: '1px solid var(--border-strong)', borderRadius: 18, boxShadow: 'var(--shadow-lg)', padding: '20px 24px' }} onClick={e => e.stopPropagation()}>
        <div style={{ fontWeight: 600, marginBottom: 14 }}>移动到文件夹</div>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 4, maxHeight: 300, overflowY: 'auto' }}>
          <button className="btn btn-sm" style={{ justifyContent: 'flex-start', gap: 8, background: file.folder_id === null ? 'var(--ember-tint)' : '' }} onClick={() => onMove(null)}>
            <Icon name="layers" size={13} />根目录
          </button>
          {folders.map(f => (
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
  const [toastMessage, setToastMessage] = useState('');
  const [searchQuery, setSearchQuery] = useState('');
  const [searchResults, setSearchResults] = useState<MaterialSearchResult[]>([]);
  const [searching, setSearching] = useState(false);
  const [aiSearching, setAiSearching] = useState(false);
  const [searchMode, setSearchMode] = useState<'quick' | 'ai'>('quick');

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
  const [confirmDelFolder, setConfirmDelFolder] = useState<MaterialFolder | null>(null);
  const [confirmDelFile,   setConfirmDelFile]   = useState<MaterialFile | null>(null);

  const fileInputRef = useRef<HTMLInputElement>(null);
  const searchSeqRef = useRef(0);
  const toastTimerRef = useRef<number | null>(null);
  const visibleFolders = visibleMaterialFolders(folders);
  const trimmedSearch = searchQuery.trim();
  const searchActive = trimmedSearch.length > 0;
  const selectedFolderTreeIds = selectedFolderId ? folderTreeIds(selectedFolderId, visibleFolders) : null;
  const displayedFiles = selectedFolderId === null
    ? files
    : files.filter(f => f.folder_id !== null && selectedFolderTreeIds?.has(f.folder_id));
  const rootFileCount = files.length;
  const newFolderNameError = newFolderName.trim()
    ? validateProjectEntryName(newFolderName, '文件夹名称')
    : null;
  const renameFolderNameError = renameName.trim()
    ? validateProjectEntryName(renameName, '文件夹名称')
    : null;

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

  useEffect(() => {
    const query = searchQuery.trim();
    const seq = ++searchSeqRef.current;
    if (!query) {
      setSearchResults([]);
      setSearching(false);
      setAiSearching(false);
      setSearchMode('quick');
      return;
    }

    setSearching(true);
    setAiSearching(false);
    const timer = window.setTimeout(async () => {
      try {
        const results = await searchMaterialFiles(projectId, query, false);
        if (searchSeqRef.current === seq) {
          setSearchResults(results);
          setSearchMode('quick');
        }
      } catch (e) {
        if (searchSeqRef.current === seq) setError(String(e));
      } finally {
        if (searchSeqRef.current === seq) setSearching(false);
      }
    }, 260);

    return () => window.clearTimeout(timer);
  }, [projectId, searchQuery, files.length]);

  const flash = (msg: string) => { setMessage(msg); setTimeout(() => setMessage(''), 4000); };
  const toast = (msg: string) => {
    setToastMessage(msg);
    if (toastTimerRef.current !== null) window.clearTimeout(toastTimerRef.current);
    toastTimerRef.current = window.setTimeout(() => setToastMessage(''), 4200);
  };

  useEffect(() => {
    return () => {
      if (toastTimerRef.current !== null) window.clearTimeout(toastTimerRef.current);
    };
  }, []);

  const handleFiles = async (fileList: FileList) => {
    setUploading(true); setError('');
    const invalidFiles: string[] = [];
    for (const f of Array.from(fileList)) {
      const nameError = validateProjectEntryName(f.name, '文件名');
      if (nameError) {
        invalidFiles.push(`${f.name}：${nameError}`);
        continue;
      }

      try {
        const buf = await f.arrayBuffer();
        const b64 = btoa(String.fromCharCode(...new Uint8Array(buf)));
        await importMaterialFile(projectId, selectedFolderId, f.name, f.type, b64);
      } catch (e) { setError(String(e)); }
    }
    if (invalidFiles.length > 0) {
      const shown = invalidFiles.slice(0, 3).join('；');
      setError(`已跳过 ${invalidFiles.length} 个名称不合法的文件：${shown}${invalidFiles.length > 3 ? '…' : ''}`);
    }
    setUploading(false);
    await load();
  };

  const doCreateFolder = async () => {
    if (!newFolderName.trim()) return;
    const nameError = validateProjectEntryName(newFolderName, '文件夹名称');
    if (nameError) { setError(nameError); return; }
    try {
      await createMaterialFolder(projectId, creatingFolder?.parentId ?? null, newFolderName.trim());
      setCreatingFolder(null); setNewFolderName('');
      await load();
    } catch (e) { setError(String(e)); }
  };

  const doRenameFolder = async () => {
    if (!renamingFolder || !renameName.trim()) return;
    const nameError = validateProjectEntryName(renameName, '文件夹名称');
    if (nameError) { setError(nameError); return; }
    try {
      await renameMaterialFolder(renamingFolder.id, renameName.trim());
      setRenamingFolder(null); setRenameName('');
      await load();
    } catch (e) { setError(String(e)); }
  };

  const doDeleteFolder = async (f: MaterialFolder) => {
    setConfirmDelFolder(f);
  };

  const execDeleteFolder = async (f: MaterialFolder) => {
    setConfirmDelFolder(null);
    try {
      const ok = await deleteMaterialFolder(f.id);
      if (!ok) throw new Error('文件夹不存在或已被删除');
      if (selectedFolderId === f.id) setSelectedFolderId(null);
      await load();
    } catch (e) { setError(String(e)); }
  };

  const doDeleteFile = async (f: MaterialFile) => {
    setConfirmDelFile(f);
  };

  const execDeleteFile = async (f: MaterialFile) => {
    setConfirmDelFile(null);
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
    searchSeqRef.current += 1;
    setSearchQuery('');
    setSearchResults([]);
    setSearchMode('quick');
    setSearching(false);
    setAiSearching(false);
    try {
      await load();
      setSelectedFolderId(null);
      toast('物料库已校准');
    }
    catch (e) { setError(String(e)); }
    finally { setRefreshing(false); }
  };

  const doAiOrganize = async () => {
    setAiWorking(true); setError(''); setMessage('');
    try { toast(await aiOrganizeMaterials(projectId)); await load(); }
    catch (e) { setError(String(e)); }
    finally { setAiWorking(false); }
  };

  const doAiSearch = async () => {
    const query = searchQuery.trim();
    if (!query) return;
    const seq = ++searchSeqRef.current;
    setAiSearching(true); setSearching(false); setError('');
    try {
      const results = await searchMaterialFiles(projectId, query, true);
      if (searchSeqRef.current === seq) {
        setSearchResults(results);
        setSearchMode('ai');
      }
    } catch (e) {
      if (searchSeqRef.current === seq) setError(String(e));
    } finally {
      if (searchSeqRef.current === seq) setAiSearching(false);
    }
  };

  const doBackup = async () => {
    setBackupWorking(true); setError('');
    try { flash(await backupMaterialFiles(projectId, [])); await load(); }
    catch (e) { setError(String(e)); }
    finally { setBackupWorking(false); }
  };

  const rootFolders = buildTree(visibleFolders, null);

  return (
    <div style={{ flex: 1, display: 'flex', flexDirection: 'column', overflow: 'hidden', minHeight: 0 }}>
      {toastMessage && (
        <div
          role="status"
          style={{
            position: 'fixed',
            right: 24,
            bottom: 24,
            zIndex: 260,
            maxWidth: 360,
            display: 'flex',
            alignItems: 'flex-start',
            gap: 10,
            padding: '11px 13px',
            borderRadius: 8,
            border: '1px solid var(--border-strong)',
            background: 'var(--bg-2)',
            boxShadow: 'var(--shadow-lg)',
            color: 'var(--text-2)',
            fontSize: 'var(--text-label)',
            lineHeight: 'var(--leading-normal)',
          }}
        >
          <Icon name="brain" size={15} style={{ color: 'var(--ember)', flexShrink: 0, marginTop: 1 }} />
          <span style={{ minWidth: 0, wordBreak: 'break-word' }}>{toastMessage}</span>
          <button className="icon-btn" style={{ width: 20, height: 20, flexShrink: 0 }} onClick={() => setToastMessage('')} title="关闭提示" aria-label="关闭提示">
            <Icon name="x" size={11} />
          </button>
        </div>
      )}

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
        <button
          className={'btn btn-sm' + (aiWorking ? ' btn-primary' : '')}
          style={aiWorking ? { boxShadow: '0 0 0 3px var(--ember-tint), 0 6px 18px var(--ember-tint-strong)' } : undefined}
          onClick={doAiOrganize}
          disabled={aiWorking}
          aria-busy={aiWorking}
        >
          <Icon name="brain" size={13} style={aiWorking ? { animation: 'spin 1s linear infinite' } : undefined} />{aiWorking ? 'AI 整理中…' : 'AI 整理'}
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
        <div style={{ width: 245, flexShrink: 0, borderRight: '1px solid var(--border)', overflowY: 'auto', padding: '10px 6px' }}>
          <div style={{ padding: '0 2px 10px', marginBottom: 8, borderBottom: '1px solid var(--border)' }}>
            <div className="search" style={{ width: '100%', padding: '5px 7px 5px 9px' }}>
              <Icon name={aiSearching ? 'brain' : 'search'} size={14} style={aiSearching || searching ? { color: 'var(--ember)' } : undefined} />
              <input
                value={searchQuery}
                onChange={e => setSearchQuery(e.target.value)}
                onKeyDown={e => { if (e.key === 'Enter') doAiSearch(); if (e.key === 'Escape') setSearchQuery(''); }}
                placeholder="搜索物料"
                style={{ minWidth: 0 }}
              />
              {searchQuery && (
                <button className="icon-btn" style={{ width: 22, height: 22, flexShrink: 0 }} onClick={() => setSearchQuery('')} title="清空搜索" aria-label="清空搜索">
                  <Icon name="x" size={11} />
                </button>
              )}
              <button className="btn btn-sm" style={{ padding: '3px 6px', minWidth: 34, height: 24, flexShrink: 0, justifyContent: 'center' }} onClick={doAiSearch} disabled={!trimmedSearch || aiSearching} title="AI 查找">
                <Icon name="brain" size={11} />{aiSearching ? '' : 'AI'}
              </button>
            </div>
          </div>

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
                {newFolderNameError && <div style={{ marginTop: 5, color: 'var(--red)', fontSize: 'var(--text-caption)' }}>{newFolderNameError}</div>}
              </div>
              <div style={{ display: 'flex', gap: 4, marginTop: 5 }}>
                <button className="btn btn-sm btn-primary" style={{ flex: 1, fontSize: 'var(--text-caption)' }} onClick={doCreateFolder} disabled={!newFolderName.trim() || Boolean(newFolderNameError)}>确认</button>
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
          ) : searchActive ? (
            searchResults.length === 0 ? (
              <div className="empty" style={{ minHeight: 180 }}>
                <Icon name={aiSearching ? 'brain' : 'search'} size={34} style={{ opacity: .25 }} />
                <div>{aiSearching || searching ? '正在查找文件…' : '没有找到匹配文件'}</div>
                <div style={{ fontSize: 'var(--text-caption)', color: 'var(--text-faint)' }}>可尝试描述文件用途，例如“登录接口说明”或“客户访谈记录”。</div>
              </div>
            ) : (
              <div style={{ display: 'flex', flexDirection: 'column', gap: 8, minWidth: 0 }}>
                <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 12, padding: '0 2px 5px' }}>
                  <div style={{ fontSize: 'var(--text-label)', color: 'var(--text-3)' }}>
                    {searchMode === 'ai' ? 'AI 查找结果' : '快速检索结果'} · {searchResults.length} 个文件
                  </div>
                  {searching && <div style={{ fontSize: 'var(--text-caption)', color: 'var(--text-faint)' }}>正在更新…</div>}
                </div>
                {searchResults.map(r => (
                  <FileCard key={r.file.id} file={r.file}
                    searchMeta={{ folderPath: r.folder_path, reason: r.match_reason, preview: r.content_preview }}
                    onOpen={() => openMaterialFile(r.file.id).catch(e => setError(String(e)))}
                    onDelete={() => doDeleteFile(r.file)}
                    onMove={() => setMovingFile(r.file)}
                  />
                ))}
              </div>
            )
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
        <div style={{ position: 'fixed', inset: 'var(--win-gutter,0)', borderRadius: 14, background: 'rgba(0,0,0,.5)', backdropFilter: 'blur(3px)', display: 'grid', placeItems: 'center', zIndex: 220 }}>
          <div style={{ width: 340, background: 'var(--bg-2)', border: '1px solid var(--border-strong)', borderRadius: 18, boxShadow: 'var(--shadow-lg)', padding: '20px 24px' }} onClick={e => e.stopPropagation()}>
            <div style={{ fontWeight: 600, marginBottom: 12 }}>重命名文件夹</div>
            <div className="field" style={{ marginBottom: 12 }}>
              <input autoFocus value={renameName} onChange={e => setRenameName(e.target.value)}
                onKeyDown={e => { if (e.key === 'Enter') doRenameFolder(); if (e.key === 'Escape') setRenamingFolder(null); }} />
              {renameFolderNameError && <div style={{ marginTop: 5, color: 'var(--red)', fontSize: 'var(--text-caption)' }}>{renameFolderNameError}</div>}
            </div>
            <div style={{ display: 'flex', gap: 8, justifyContent: 'flex-end' }}>
              <button className="btn btn-sm" onClick={() => setRenamingFolder(null)}>取消</button>
              <button className="btn btn-sm btn-primary" onClick={doRenameFolder} disabled={!renameName.trim() || Boolean(renameFolderNameError)}>确认</button>
            </div>
          </div>
        </div>
      )}

      {/* create subfolder modal */}
      {creatingFolder !== null && creatingFolder.parentId !== null && (
        <div style={{ position: 'fixed', inset: 'var(--win-gutter,0)', borderRadius: 14, background: 'rgba(0,0,0,.5)', backdropFilter: 'blur(3px)', display: 'grid', placeItems: 'center', zIndex: 220 }}>
          <div style={{ width: 340, background: 'var(--bg-2)', border: '1px solid var(--border-strong)', borderRadius: 18, boxShadow: 'var(--shadow-lg)', padding: '20px 24px' }} onClick={e => e.stopPropagation()}>
            <div style={{ fontWeight: 600, marginBottom: 12 }}>新建子文件夹</div>
            <div className="field" style={{ marginBottom: 12 }}>
              <input autoFocus placeholder="文件夹名称" value={newFolderName}
                onChange={e => setNewFolderName(e.target.value)}
                onKeyDown={e => { if (e.key === 'Enter') doCreateFolder(); if (e.key === 'Escape') setCreatingFolder(null); }} />
              {newFolderNameError && <div style={{ marginTop: 5, color: 'var(--red)', fontSize: 'var(--text-caption)' }}>{newFolderNameError}</div>}
            </div>
            <div style={{ display: 'flex', gap: 8, justifyContent: 'flex-end' }}>
              <button className="btn btn-sm" onClick={() => setCreatingFolder(null)}>取消</button>
              <button className="btn btn-sm btn-primary" onClick={doCreateFolder} disabled={!newFolderName.trim() || Boolean(newFolderNameError)}>创建</button>
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

      {confirmDelFolder && (
        <ConfirmModal
          msg={`确认删除文件夹「${confirmDelFolder.name}」？`}
          sub="文件夹及其中所有文件将移入系统回收站，可从回收站恢复。"
          okLabel="移入回收站"
          onOk={() => execDeleteFolder(confirmDelFolder)}
          onCancel={() => setConfirmDelFolder(null)}
        />
      )}

      {confirmDelFile && (
        <ConfirmModal
          msg={`确认删除「${confirmDelFile.original_name}」？`}
          sub="文件将移入系统回收站，可从回收站恢复。"
          okLabel="移入回收站"
          onOk={() => execDeleteFile(confirmDelFile)}
          onCancel={() => setConfirmDelFile(null)}
        />
      )}
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
  { id: 'reference',    label: '参考',     icon: 'file',    hint: 'Agent 写入的技术方案、设计文档等自由参考资料' },
];

// 注入档位元信息：标签 + chip 语义色（绿=常驻/蓝=按需/灰=关闭）。
const INJECTION_META: { id: SpecInjection; label: string; chip: string; hint: string }[] = [
  { id: 'always',    label: '常驻', chip: 'green', hint: '全文随每次任务注入上下文' },
  { id: 'on_demand', label: '按需', chip: 'blue',  hint: '仅列入清单，Agent 用 read_spec 工具按需读取' },
  { id: 'off',       label: '关闭', chip: '',      hint: '不向 Agent 暴露' },
];

function SpecCard({ spec, onEdit, onDelete, onInjection }: {
  spec: ProjectSpec;
  onEdit: (s: ProjectSpec) => void;
  onDelete: (s: ProjectSpec) => void;
  onInjection: (s: ProjectSpec, mode: SpecInjection) => void;
}) {
  const [hovered, setHovered] = useState(false);
  const isFile = spec.source === 'file';
  // file 源的正文不在 list 里（仅描述）；db 源直接展示 content。
  const preview = isFile ? spec.description : (spec.content || spec.description);
  return (
    <div
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
      style={{ background: 'var(--bg-3)', border: '1px solid var(--border)', borderRadius: 10, padding: '11px 14px', position: 'relative' }}
    >
      <div style={{ display: 'flex', alignItems: 'flex-start', gap: 8 }}>
        <div style={{ flex: 1, minWidth: 0 }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 6, marginBottom: 4, flexWrap: 'wrap' }}>
            <span style={{ fontSize: 'var(--text-control)', fontWeight: 600, color: 'var(--text-1)' }}>{spec.title}</span>
            <span className={`chip ${isFile ? 'amber' : ''}`} style={{ fontSize: 'var(--text-micro)' }}>
              <Icon name={isFile ? 'file' : 'box'} size={9} />{isFile ? '文件' : 'DB'}
            </span>
          </div>
          {preview && (
            <div style={{ fontSize: 'var(--text-label)', color: 'var(--text-3)', lineHeight: 'var(--leading-relaxed)', whiteSpace: 'pre-wrap', wordBreak: 'break-word' }}>{preview}</div>
          )}
          {isFile && (
            <div style={{ fontSize: 'var(--text-micro)', color: 'var(--text-faint)', fontFamily: 'var(--font-mono)', marginTop: 4 }}>.autoforge/{spec.rel_path}</div>
          )}
        </div>
        {hovered && (
          <div style={{ display: 'flex', gap: 3, flexShrink: 0 }}>
            <button className="btn btn-sm" style={{ padding: '2px 6px' }} onClick={() => onEdit(spec)} title={isFile ? '查看 / 编辑文件内容' : '编辑'}>
              <Icon name="edit" size={11} />
            </button>
            <button className="btn btn-sm" style={{ padding: '2px 6px', color: 'var(--red)' }} onClick={() => onDelete(spec)} title={isFile ? '删除（连带磁盘文件）' : '删除'}>
              <Icon name="trash" size={11} />
            </button>
          </div>
        )}
      </div>
      {/* 注入档位：行内即时切换（DESIGN seg 段控，替代下拉） */}
      <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginTop: 9 }}>
        <span style={{ fontSize: 'var(--text-micro)', color: 'var(--text-faint)', fontFamily: 'var(--font-mono)', letterSpacing: '.08em', textTransform: 'uppercase' }}>注入</span>
        <div className="seg" style={{ alignSelf: 'flex-start' }}>
          {INJECTION_META.map(m => (
            <button key={m.id} type="button" className={spec.injection === m.id ? 'on' : ''}
              title={m.hint} onClick={() => { if (spec.injection !== m.id) onInjection(spec, m.id); }}
              style={{ fontSize: 'var(--text-micro)', padding: '2px 9px' }}>
              {m.label}
            </button>
          ))}
        </div>
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
  const isFile = spec?.source === 'file';
  const [title, setTitle]       = useState(spec?.title ?? '');
  const [content, setContent]   = useState(spec?.content ?? '');
  const [description, setDesc]  = useState(spec?.description ?? '');
  const [cat, setCat]           = useState<SpecCategory>(spec?.category ?? category);
  const [injection, setInj]     = useState<SpecInjection>(spec?.injection ?? 'always');
  const [saving, setSaving]     = useState(false);
  const [loading, setLoading]   = useState(isFile);
  const [err, setErr]           = useState('');

  // file 源正文不在 list 里，进入时按需拉全文。
  useEffect(() => {
    if (!isFile || !spec) return;
    let alive = true;
    setLoading(true);
    getSpecContent(spec.id)
      .then(c => { if (alive) setContent(c); })
      .catch(e => { if (alive) setErr(String(e)); })
      .finally(() => { if (alive) setLoading(false); });
    return () => { alive = false; };
  }, [isFile, spec]);

  const save = async () => {
    setSaving(true); setErr('');
    try {
      const result = await upsertProjectSpec(projectId, spec?.id ?? null, cat, title, content, description, injection);
      onSave(result);
    } catch (e) { setErr(String(e)); }
    finally { setSaving(false); }
  };

  const catMeta = SPEC_CATEGORIES.find(c => c.id === cat)!;

  return (
    <div style={{ position: 'fixed', inset: 'var(--win-gutter,0)', borderRadius: 14, background: 'rgba(0,0,0,.5)', backdropFilter: 'blur(3px)', display: 'grid', placeItems: 'center', zIndex: 220 }}>
      <div style={{ width: 560, maxHeight: '86vh', overflowY: 'auto', background: 'var(--bg-2)', border: '1px solid var(--border-strong)', borderRadius: 18, boxShadow: 'var(--shadow-lg)', padding: '22px 24px' }} onClick={e => e.stopPropagation()}>
        <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 18 }}>
          <div className="eyebrow" style={{ fontSize: 'var(--text-control)' }}>
            <span className="en">{catMeta.label.toUpperCase()}</span>
            <span className="cn"> · {spec ? (isFile ? '编辑文件规格' : '编辑规格') : '新增规格'}</span>
          </div>
          <button className="icon-btn" onClick={onClose}><Icon name="x" size={16} /></button>
        </div>
        {err && <div style={{ color: 'var(--red)', fontSize: 'var(--text-label)', marginBottom: 12, padding: '6px 10px', background: 'rgba(219,90,64,.08)', borderRadius: 6 }}>{err}</div>}
        {isFile && (
          <div style={{ fontSize: 'var(--text-micro)', color: 'var(--text-faint)', fontFamily: 'var(--font-mono)', marginBottom: 12 }}>
            <Icon name="file" size={10} /> .autoforge/{spec?.rel_path} · 保存将写回此文件
          </div>
        )}
        <div className="field" style={{ marginBottom: 14 }}>
          <label>标题</label>
          <input autoFocus={!isFile} placeholder="简短的规格名称" value={title} onChange={e => setTitle(e.target.value)}
            onKeyDown={e => { if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) save(); if (e.key === 'Escape') onClose(); }} />
        </div>
        <div className="field" style={{ marginBottom: 14 }}>
          <label>描述 <span style={{ fontWeight: 400, color: 'var(--text-faint)', fontSize: 'var(--text-caption)' }}>（清单/工具里展示的一句话摘要）</span></label>
          <input placeholder="一句话说明这条规格是关于什么的" value={description} onChange={e => setDesc(e.target.value)} />
        </div>
        <div className="field" style={{ marginBottom: 14 }}>
          <label>分类</label>
          <div className="seg" style={{ alignSelf: 'flex-start', flexWrap: 'wrap' }}>
            {SPEC_CATEGORIES.map(c => (
              <button key={c.id} type="button" className={cat === c.id ? 'on' : ''} onClick={() => setCat(c.id)}
                style={{ fontSize: 'var(--text-micro)', padding: '3px 10px' }}>{c.label}</button>
            ))}
          </div>
        </div>
        <div className="field" style={{ marginBottom: 14 }}>
          <label>注入档位</label>
          <div className="seg" style={{ alignSelf: 'flex-start' }}>
            {INJECTION_META.map(m => (
              <button key={m.id} type="button" className={injection === m.id ? 'on' : ''} title={m.hint} onClick={() => setInj(m.id)}
                style={{ fontSize: 'var(--text-micro)', padding: '3px 12px' }}>{m.label}</button>
            ))}
          </div>
          <span style={{ fontSize: 'var(--text-micro)', color: 'var(--text-faint)', marginTop: 5 }}>{INJECTION_META.find(m => m.id === injection)?.hint}</span>
        </div>
        <div className="field" style={{ marginBottom: 20 }}>
          <label>内容 <span style={{ fontWeight: 400, color: 'var(--text-faint)', fontSize: 'var(--text-caption)' }}>（{catMeta.hint}）</span></label>
          <textarea rows={isFile ? 12 : 5} placeholder={loading ? '读取文件中…' : '具体约束说明…'} value={content} onChange={e => setContent(e.target.value)} disabled={loading}
            style={{ resize: 'vertical', fontFamily: 'var(--font-mono)', fontSize: 'var(--text-label)' }} />
        </div>
        <div style={{ display: 'flex', justifyContent: 'flex-end', gap: 8 }}>
          <button className="btn btn-sm" onClick={onClose}>取消</button>
          <button className="btn btn-sm btn-primary" onClick={save} disabled={saving || loading || !title.trim()}>
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
  const [confirmAiGen,  setConfirmAiGen]  = useState(false);
  const [confirmDelSpec, setConfirmDelSpec] = useState<ProjectSpec | null>(null);
  const [scanning, setScanning]   = useState(false);

  const load = useCallback(async () => {
    try {
      setSpecs(await listProjectSpecs(projectId));
    } catch (e) { setError(String(e)); }
    finally { setLoading(false); }
  }, [projectId]);

  const flash = (msg: string) => { setMessage(msg); setTimeout(() => setMessage(''), 4000); };

  // 进入面板时先对账 .autoforge/specs 目录（把 agent 写的自由文件登记为文件规格），再加载。
  useEffect(() => {
    let alive = true;
    setLoading(true);
    (async () => {
      try { await scanSpecFiles(projectId); } catch { /* 对账失败不阻断展示 */ }
      if (alive) await load();
    })();
    return () => { alive = false; };
  }, [projectId, load]);

  const doScan = async () => {
    setScanning(true); setError('');
    try { flash(await scanSpecFiles(projectId)); await load(); }
    catch (e) { setError(String(e)); }
    finally { setScanning(false); }
  };

  const onInjection = async (s: ProjectSpec, mode: SpecInjection) => {
    // 乐观更新，再落库。
    setSpecs(prev => prev.map(x => x.id === s.id ? { ...x, injection: mode } : x));
    try { await setSpecInjection(s.id, mode); }
    catch (e) { setError(String(e)); await load(); }
  };

  const doAiGenerate = () => { setConfirmAiGen(true); };

  const execAiGenerate = async () => {
    setConfirmAiGen(false);
    setAiWorking(true); setError('');
    try { flash(await aiGenerateSpecs(projectId)); await load(); }
    catch (e) { setError(String(e)); }
    finally { setAiWorking(false); }
  };

  const doDelete = (s: ProjectSpec) => { setConfirmDelSpec(s); };

  const execDeleteSpec = async (s: ProjectSpec) => {
    setConfirmDelSpec(null);
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
        <button className="btn btn-sm" onClick={doScan} disabled={scanning} title="扫描 .autoforge/specs 目录，登记 Agent 写入的文件规格">
          <Icon name="refresh" size={13} style={scanning ? { animation: 'spin 1s linear infinite' } : undefined} />
          {scanning ? '扫描中…' : '重新扫描'}
        </button>
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
                <SpecCard key={s.id} spec={s} onEdit={s => setEditing(s)} onDelete={doDelete} onInjection={onInjection} />
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

      {confirmAiGen && (
        <ConfirmModal
          msg="AI 一键生成规格"
          sub="AI 将分析项目信息并重新生成所有分类规格，现有规格将被覆盖。确认继续？"
          okLabel="生成"
          danger={false}
          onOk={execAiGenerate}
          onCancel={() => setConfirmAiGen(false)}
        />
      )}

      {confirmDelSpec && (
        <ConfirmModal
          msg={`确认删除规格「${confirmDelSpec.title}」？`}
          sub={confirmDelSpec.source === 'file' ? '这是文件规格，将同时删除磁盘上的 .autoforge/specs/ 文件，不可恢复。' : undefined}
          okLabel="删除"
          onOk={() => execDeleteSpec(confirmDelSpec)}
          onCancel={() => setConfirmDelSpec(null)}
        />
      )}
    </div>
  );
}


// ── ConfigPanel（运行配置）────────────────────────────────────────────────────

const MASK_RULE_OPTS: { value: MaskRule; label: string }[] = [
  { value: 'mask', label: '掩码 (首字符 + ****)' },
  { value: 'hash', label: '哈希 (不可逆随机)' },
  { value: 'drop', label: '清空 (置 NULL)' },
];

function CfgField({ label, hint, value, onChange, placeholder, mono = true, full = false }: {
  label: string; hint?: string; value: string;
  onChange: (v: string) => void; placeholder?: string; mono?: boolean; full?: boolean;
}) {
  return (
    <div className={`field${full ? ' full' : ''}`}>
      <label>{label}</label>
      <input value={value} onChange={e => onChange(e.target.value)} placeholder={placeholder}
        style={mono ? { fontFamily: 'var(--font-mono)' } : undefined} />
      {hint && <div style={{ fontSize: 'var(--text-micro)', color: 'var(--text-faint)', marginTop: 4, lineHeight: 'var(--leading-normal)' }}>{hint}</div>}
    </div>
  );
}

function CfgSection({ title, desc, children }: { title: string; desc?: string; children: React.ReactNode }) {
  return (
    <div style={{ marginBottom: 22 }}>
      <div style={{ fontSize: 'var(--text-caption)', letterSpacing: '.08em', textTransform: 'uppercase', color: 'var(--text-faint)', fontWeight: 600, marginBottom: 3, fontFamily: 'var(--font-mono)' }}>{title}</div>
      {desc && <div style={{ fontSize: 'var(--text-label)', color: 'var(--text-3)', marginBottom: 10, lineHeight: 'var(--leading-normal)' }}>{desc}</div>}
      {children}
    </div>
  );
}

function ConfigPanel({ project, onSaved }: { project: Project; onSaved: () => void | Promise<void> }) {
  const [form, setForm] = useState<ProjectConfigForm>(() => parseProjectConfigForm(project.config_yaml));
  const [saving, setSaving] = useState(false);
  const [aiWorking, setAiWorking] = useState(false);
  const [message, setMessage] = useState('');
  const [error, setError] = useState('');
  // 自动检测出的应用品类（只读展示，区别于可选的"预览方式"）。
  const [category, setCategory] = useState<string>('');

  useEffect(() => {
    let alive = true;
    detectProjectCategory(project.id).then(c => { if (alive) setCategory(c); }).catch(() => {});
    return () => { alive = false; };
  }, [project.id]);

  const set = (patch: Partial<ProjectConfigForm>) => setForm(f => ({ ...f, ...patch }));
  const flash = (m: string) => { setMessage(m); setTimeout(() => setMessage(''), 5000); };

  // AI 推断填表：仅覆盖 AI 给出非空值的字段，保留人工已填内容（脱敏字段不由 AI 生成）。
  const applyDraft = (d: RunConfigDraft) => {
    const pick = (v: string | null | undefined, cur: string) => (v == null || v === '' ? cur : v);
    const num = (v: number | null | undefined, cur: string) => (v == null ? cur : String(v));
    setForm(f => ({
      ...f,
      devKind: d.dev_kind === 'tauri' ? 'tauri' : d.dev_kind === 'miniapp' ? 'miniapp' : d.dev_kind === 'web' ? 'web' : f.devKind,
      devCommand: pick(d.dev_command, f.devCommand),
      appCommand: pick(d.app_command, f.appCommand),
      testUnit: pick(d.test_unit, f.testUnit),
      testUnitTimeout: num(d.test_unit_timeout, f.testUnitTimeout),
      testIntegration: pick(d.test_integration, f.testIntegration),
      testIntegrationTimeout: num(d.test_integration_timeout, f.testIntegrationTimeout),
      qualityLint: pick(d.quality_lint, f.qualityLint),
      qualityTyping: pick(d.quality_typing, f.qualityTyping),
      qualitySecurity: pick(d.quality_security, f.qualitySecurity),
      projectLanguage: pick(d.project_language, f.projectLanguage),
      projectFramework: pick(d.project_framework, f.projectFramework),
      previewBuild: pick(d.preview_build, f.previewBuild),
      previewStart: pick(d.preview_start, f.previewStart),
      deployCommand: pick(d.deploy_command, f.deployCommand),
    }));
  };

  const aiGenerate = async () => {
    setAiWorking(true); setError('');
    try {
      applyDraft(await aiGenerateRunConfig(project.id));
      flash('已根据仓库推断填入，请检查后点「保存配置」');
    } catch (e) { setError(String(e)); }
    finally { setAiWorking(false); }
  };
  const setRow = (i: number, patch: Partial<SensitiveFieldRow>) =>
    setForm(f => ({ ...f, sensitiveFields: f.sensitiveFields.map((r, idx) => idx === i ? { ...r, ...patch } : r) }));
  const addRow = () => setForm(f => ({ ...f, sensitiveFields: [...f.sensitiveFields, { table: '', fields: '', rule: 'mask' }] }));
  const delRow = (i: number) => setForm(f => ({ ...f, sensitiveFields: f.sensitiveFields.filter((_, idx) => idx !== i) }));

  const save = async () => {
    setSaving(true); setError('');
    try {
      const json = buildProjectConfig(form, project.config_yaml);
      await updateProject(project.id, { config_yaml: json ?? '' });
      flash('运行配置已保存');
      await onSaved();
    } catch (e) { setError(String(e)); }
    finally { setSaving(false); }
  };

  return (
    <div style={{ flex: 1, display: 'flex', flexDirection: 'column', overflow: 'hidden', minHeight: 0 }}>
      {/* action bar */}
      <div style={{ display: 'flex', alignItems: 'center', gap: 6, padding: '10px 20px', borderBottom: '1px solid var(--border)', flexShrink: 0 }}>
        <div style={{ fontSize: 'var(--text-label)', color: 'var(--text-3)' }}>预览 / 测试 / 巡检 / 部署 / 脱敏均读取此处配置</div>
        <div style={{ flex: 1 }} />
        {message && <span style={{ fontSize: 'var(--text-label)', color: 'var(--green)', maxWidth: 320, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{message}</span>}
        <button className="btn btn-sm" onClick={aiGenerate} disabled={aiWorking || saving} title="读取仓库构建文件，AI 推断并填入运行配置">
          <Icon name="brain" size={13} style={aiWorking ? { animation: 'spin 1s linear infinite' } : undefined} />
          {aiWorking ? 'AI 推断中…' : 'AI 一键生成'}
        </button>
        <button className="btn btn-sm btn-primary" onClick={save} disabled={saving || aiWorking}>
          <Icon name="check" size={13} />{saving ? '保存中…' : '保存配置'}
        </button>
      </div>

      {error && (
        <div style={{ padding: '6px 20px', color: 'var(--red)', fontSize: 'var(--text-label)', background: 'rgba(219,90,64,.06)', flexShrink: 0 }}>{error}</div>
      )}

      <div style={{ flex: 1, overflowY: 'auto', padding: '18px 22px' }}>
        <CfgSection title="审计预览环境" desc="变更审核时拉起预览实例所需。预览固定加载 http://localhost:{port}，命令中的 {port} 会按变更替换为独立端口。">
          <div className="field" style={{ marginBottom: 10 }}>
            <label>应用类型</label>
            {/* 自动检测出的品类，只读；区别于下方可选的"预览方式"。 */}
            <div style={{ display: 'flex', alignItems: 'center', gap: 6, marginBottom: 8 }}>
              <span style={{ fontSize: 'var(--text-caption)', color: 'var(--text-3)' }}>检测到品类</span>
              <span className="chip ember" style={{ fontSize: 'var(--text-micro)' }}>{category || '检测中…'}</span>
              <span style={{ fontSize: 'var(--text-micro)', color: 'var(--text-faint)' }}>（自动嗅探仓库，仅供参考）</span>
            </div>
            <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
              <span style={{ fontSize: 'var(--text-caption)', color: 'var(--text-3)', fontFamily: 'var(--font-mono)' }}>预览方式</span>
              <div className="seg" style={{ alignSelf: 'flex-start' }}>
                <button type="button" className={form.devKind === 'web' ? 'on' : ''} onClick={() => set({ devKind: 'web' })}>Web（浏览器）</button>
                <button type="button" className={form.devKind === 'tauri' ? 'on' : ''} onClick={() => set({ devKind: 'tauri' })}>Tauri 桌面</button>
                <button type="button" className={form.devKind === 'miniapp' ? 'on' : ''} onClick={() => set({ devKind: 'miniapp' })}>微信小程序</button>
              </div>
            </div>
            <span style={{ fontSize: 'var(--text-caption)', color: 'var(--text-faint)', marginTop: 4 }}>
              {form.devKind === 'web' && '浏览器预览：前端 / 后端服务 / 静态站均在 localhost:{port} 起 dev server 后用浏览器打开。'}
              {form.devKind === 'tauri' && '桌面程序：直接启动原生窗口（可访问完整 IPC），不走 iframe。'}
              {form.devKind === 'miniapp' && '微信小程序：无本地 server，预览=一次性编译产物，用微信开发者工具打开（可在「设置 → 小程序预览」配 CLI 自动打开）。'}
            </span>
          </div>
          <div className="cfg-fields" style={{ gridTemplateColumns: '1fr 1fr' }}>
            <CfgField full
              label={form.devKind === 'tauri' ? '前端预览启动命令' : form.devKind === 'miniapp' ? '编译命令' : '启动命令'}
              value={form.devCommand} onChange={v => set({ devCommand: v })}
              placeholder={form.devKind === 'miniapp' ? 'npm run build:weapp' : 'npm run dev -- --port {port}'} />
            {form.devKind === 'tauri' && (
              <CfgField full label="桌面应用启动命令（可选，逃生口）" value={form.appCommand} onChange={v => set({ appCommand: v })} placeholder="npm run tauri:dev" />
            )}
          </div>
        </CfgSection>

        <CfgSection title="测试套件" desc="主动巡检与合并前自动测试依次执行；任一失败将阻断合并 / 建为需求。">
          <div className="cfg-fields" style={{ gridTemplateColumns: '3fr 1fr' }}>
            <CfgField label="单元测试命令" value={form.testUnit} onChange={v => set({ testUnit: v })} placeholder="npm test" />
            <CfgField mono={false} label="超时(秒)" value={form.testUnitTimeout} onChange={v => set({ testUnitTimeout: v })} placeholder="120" />
            <CfgField label="集成测试命令" value={form.testIntegration} onChange={v => set({ testIntegration: v })} placeholder="npm run test:integration" />
            <CfgField mono={false} label="超时(秒)" value={form.testIntegrationTimeout} onChange={v => set({ testIntegrationTimeout: v })} placeholder="300" />
          </div>
        </CfgSection>

        <CfgSection title="质量检查" desc="同样纳入巡检 / 测试套件，固定 120s 超时。">
          <div className="cfg-fields" style={{ gridTemplateColumns: '1fr 1fr 1fr' }}>
            <CfgField label="Lint" value={form.qualityLint} onChange={v => set({ qualityLint: v })} placeholder="eslint ." />
            <CfgField label="类型检查" value={form.qualityTyping} onChange={v => set({ qualityTyping: v })} placeholder="tsc --noEmit" />
            <CfgField label="安全扫描" value={form.qualitySecurity} onChange={v => set({ qualitySecurity: v })} placeholder="npm audit" />
          </div>
        </CfgSection>

        <CfgSection title="部署" desc="生成部署脚本时使用；技术栈用于提示 LLM 润色，preview/deploy 命令决定脚本主体。">
          <div className="cfg-fields" style={{ gridTemplateColumns: '1fr 1fr' }}>
            <CfgField mono={false} label="语言" value={form.projectLanguage} onChange={v => set({ projectLanguage: v })} placeholder="typescript" />
            <CfgField mono={false} label="框架" value={form.projectFramework} onChange={v => set({ projectFramework: v })} placeholder="react" />
            <CfgField full label="构建命令 (preview.build)" value={form.previewBuild} onChange={v => set({ previewBuild: v })} placeholder="npm run build" />
            <CfgField full label="启动命令 (preview.start)" value={form.previewStart} onChange={v => set({ previewStart: v })} placeholder="npm run start" />
            <CfgField full label="部署命令 (deploy.command，优先于上面两条)" value={form.deployCommand} onChange={v => set({ deployCommand: v })} placeholder="./scripts/deploy.sh" />
          </div>
        </CfgSection>

        <CfgSection title="预览数据脱敏" desc="预览环境克隆数据库后，对敏感字段执行脱敏（仅 SQLite）。">
          <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
            {form.sensitiveFields.map((r, i) => (
              <div key={i} style={{ display: 'grid', gridTemplateColumns: '1fr 1.4fr 1.4fr auto', gap: 8, alignItems: 'center' }}>
                <input value={r.table} onChange={e => setRow(i, { table: e.target.value })} placeholder="表名 users" style={{ fontFamily: 'var(--font-mono)' }} />
                <input value={r.fields} onChange={e => setRow(i, { fields: e.target.value })} placeholder="字段，逗号分隔 email, phone" style={{ fontFamily: 'var(--font-mono)' }} />
                <Select className="sm" value={r.rule} onChange={v => setRow(i, { rule: v as MaskRule })} options={MASK_RULE_OPTS} />
                <button className="btn btn-sm btn-danger" onClick={() => delRow(i)} title="删除该行"><Icon name="trash" size={13} /></button>
              </div>
            ))}
            <button className="btn btn-sm" style={{ alignSelf: 'flex-start' }} onClick={addRow}><Icon name="plus" size={12} />添加脱敏字段</button>
          </div>
        </CfgSection>
      </div>
    </div>
  );
}

// ── ProjectInfoTab ────────────────────────────────────────────────────────────

function ProjectInfoTab({ project }: { project: Project }) {
  const [codeAgents, setCodeAgents] = useState<CodeAgentT[]>([]);
  const [agentSel, setAgentSel] = useState<string>(project.code_agent_id ?? '');
  const [agentMsg, setAgentMsg] = useState('');
  useEffect(() => { listCodeAgents().then(setCodeAgents).catch(() => setCodeAgents([])); }, []);
  useEffect(() => { setAgentSel(project.code_agent_id ?? ''); }, [project.id, project.code_agent_id]);

  const defaultAgent = codeAgents.find(a => a.is_default);
  const agentOptions = [
    { value: '', label: `跟随全局默认${defaultAgent ? `（${defaultAgent.label}）` : ''}` },
    ...codeAgents.filter(a => a.enabled).map(a => ({ value: a.id, label: `${a.label}（${a.kind}）` })),
  ];
  const onAgentChange = async (val: string) => {
    setAgentSel(val);
    setAgentMsg('');
    try { await setProjectCodeAgent(project.id, val || null); setAgentMsg('已保存'); }
    catch (e) { setAgentMsg(String(e)); }
  };

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
        <div style={{ display: 'flex', alignItems: 'center', padding: '10px 0', borderBottom: '1px solid var(--border)' }}>
          <div style={{ width: 90, fontSize: 'var(--text-label)', color: 'var(--text-faint)', flexShrink: 0 }}>代码 Agent</div>
          <div style={{ flex: 1, display: 'flex', alignItems: 'center', gap: 10 }}>
            <Select value={agentSel} onChange={onAgentChange} options={agentOptions} style={{ minWidth: 220 }} />
            {agentMsg && <span style={{ fontSize: 'var(--text-caption)', color: 'var(--text-3)', fontFamily: 'var(--font-mono)' }}>{agentMsg}</span>}
          </div>
        </div>
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
        <span style={{ flex: 1, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', fontSize: 'var(--text-body)', fontWeight: 600, display: 'flex', alignItems: 'center', gap: 5 }}>
          {project.is_default && <Icon name="star" size={12} style={{ color: 'var(--ember)', flexShrink: 0 }} />}
          <span style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{project.name}</span>
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

type Tab = 'info' | 'materials' | 'intake' | 'spec' | 'config';

export default function ProjectsPage({ onOpenBlueprint }: { onOpenBlueprint?: (projectId: string) => void } = {}) {
  const [projects, setProjects]         = useState<Project[]>([]);
  const [loading, setLoading]           = useState(true);
  const [error, setError]               = useState('');
  const [selectedId, setSelectedId]     = useState<string | null>(null);
  const [activeTab, setActiveTab]       = useState<Tab>('info');
  const [showCreate, setShowCreate]     = useState(false);
  const [editProject, setEditProject]   = useState<Project | null>(null);
  const [deleteTarget, setDeleteTarget] = useState<Project | null>(null);
  const [deleting, setDeleting]         = useState(false);
  const [showRecycle, setShowRecycle]   = useState(false);
  const [archived, setArchived]         = useState<Project[]>([]);
  const [recycleBusy, setRecycleBusy]   = useState(false);
  const [purgeTarget, setPurgeTarget]   = useState<Project | null>(null);

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
    listen('autoforge://event', () => load()).then(fn => { unlisten = fn; });
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

  const loadArchived = useCallback(async () => {
    try { setArchived(await listArchivedProjects()); }
    catch (e) { setError(String(e)); }
  }, []);

  const openRecycle = async () => { setShowRecycle(true); await loadArchived(); };

  const doRestore = async (project: Project) => {
    setRecycleBusy(true);
    try {
      await restoreProject(project.id);
      await loadArchived();
      await load();
      window.dispatchEvent(new Event('AutoForge:badges-refresh'));
    } catch (e) { setError(String(e)); }
    finally { setRecycleBusy(false); }
  };

  const doPurge = async () => {
    if (!purgeTarget) return;
    setRecycleBusy(true);
    try {
      await purgeProject(purgeTarget.id);
      setPurgeTarget(null);
      await loadArchived();
      window.dispatchEvent(new Event('AutoForge:badges-refresh'));
    } catch (e) { setError(String(e)); setPurgeTarget(null); }
    finally { setRecycleBusy(false); }
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

  const doSetDefault = async (project: Project) => {
    setError('');
    try {
      // 已是默认则再次点击取消默认
      await setDefaultProject(project.is_default ? '' : project.id);
      await load();
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
        <button className="btn btn-sm" style={{ marginLeft: 'auto' }} onClick={openRecycle} title="回收站：已归档项目可恢复或彻底删除">
          <Icon name="trash" size={14} />回收站
        </button>
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
                      {selectedProject.is_default && (
                        <span className="chip ember" style={{ fontSize: 'var(--text-micro)', padding: '1px 7px', display: 'inline-flex', alignItems: 'center', gap: 4 }}>
                          <Icon name="star" size={11} />默认
                        </span>
                      )}
                    </div>
                    <div style={{ fontSize: 'var(--text-label)', color: 'var(--text-faint)', fontFamily: 'var(--font-mono)', marginTop: 2 }}>
                      {selectedProject.repo_path || '仓库路径未配置'}
                    </div>
                  </div>
                  <div style={{ display: 'flex', gap: 6, flexShrink: 0 }}>
                    <button
                      className="btn btn-sm"
                      style={selectedProject.is_default ? { color: 'var(--ember)' } : undefined}
                      onClick={() => doSetDefault(selectedProject)}
                      title={selectedProject.is_default ? '取消默认项目' : '设为默认项目，其他页面将优先显示'}
                    >
                      <Icon name="star" size={13} />
                      {selectedProject.is_default ? '默认项目' : '设为默认'}
                    </button>
                    <button className="btn btn-sm" onClick={() => doToggleStatus(selectedProject)}>
                      <Icon name={selectedProject.status === 'active' ? 'pause' : 'play'} size={13} />
                      {selectedProject.status === 'active' ? '停用' : '启用'}
                    </button>
                    <button className="btn btn-sm" onClick={() => onOpenBlueprint?.(selectedProject.id)} title="到「需求孵化台」：把大需求改动炼成 PRD / 规格 / 任务并一键编码开发">
                      <Icon name="layers" size={13} />需求孵化台
                    </button>
                    <button className="btn btn-sm" onClick={() => setEditProject(selectedProject)}>
                      <Icon name="edit" size={13} />编辑
                    </button>
                    <button className="btn btn-sm" style={{ color: 'var(--red)' }} onClick={() => setDeleteTarget(selectedProject)} title="归档项目（移入回收站，数据保留）">
                      <Icon name="trash" size={13} />归档
                    </button>
                  </div>
                </div>

                {/* tabs */}
                <div style={{ display: 'flex', gap: 2, borderBottom: '1px solid var(--border)' }}>
                  {([['info', '基本信息', 'sliders'], ['materials', '物料库', 'folder'], ['intake', '需求入口', 'inbox'], ['spec', '规格', 'layers'], ['config', '运行配置', 'cpu']] as const).map(([id, label, ic]) => (
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
                {activeTab === 'intake'    && <IntakePanel key={selectedProject.id} projectId={selectedProject.id} tabOrder={['webhook', 'github', 'bulk', 'manual']} />}
                {activeTab === 'spec'      && <SpecPanel key={selectedProject.id} projectId={selectedProject.id} />}
                {activeTab === 'config'    && <ConfigPanel key={selectedProject.id} project={selectedProject} onSaved={load} />}
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
      {showRecycle && (
        <div style={{ position: 'fixed', inset: 'var(--win-gutter,0)', borderRadius: 14, background: 'rgba(0,0,0,.5)', backdropFilter: 'blur(3px)', display: 'grid', placeItems: 'center', zIndex: 230 }}>
          <div style={{ background: 'var(--bg-2)', border: '1px solid var(--border-strong)', borderRadius: 14, padding: '20px 22px', width: 560, maxHeight: '76vh', display: 'flex', flexDirection: 'column', boxShadow: 'var(--shadow-lg)' }}>
            <div style={{ display: 'flex', alignItems: 'center', marginBottom: 6 }}>
              <span style={{ fontFamily: 'var(--font-mono)', fontSize: 'var(--text-label)', letterSpacing: '.14em', textTransform: 'uppercase', color: 'var(--text-3)' }}>回收站 · 已归档项目</span>
              <button className="icon-btn" style={{ marginLeft: 'auto', width: 30, height: 30 }} onClick={() => setShowRecycle(false)} aria-label="关闭"><Icon name="x" size={15} /></button>
            </div>
            <p style={{ margin: '0 0 14px', fontSize: 'var(--text-caption)', color: 'var(--text-faint)', lineHeight: 'var(--leading-relaxed)' }}>
              归档项目保留全部数据。重新添加同一仓库会按 <code style={{ fontFamily: 'var(--font-mono)' }}>.autoforge/project.json</code> 身份锚自动挂回；也可在此恢复或彻底删除。
            </p>
            <div style={{ overflowY: 'auto', display: 'flex', flexDirection: 'column', gap: 8 }}>
              {archived.length === 0 ? (
                <div className="empty" style={{ padding: '28px 0' }}><Icon name="trash" size={28} style={{ opacity: .3 }} /><div>回收站为空</div></div>
              ) : archived.map(p => (
                <div key={p.id} className="panel" style={{ display: 'flex', alignItems: 'center', gap: 10, padding: '10px 12px' }}>
                  <div style={{ flex: 1, minWidth: 0 }}>
                    <div style={{ fontSize: 'var(--text-control)', fontWeight: 600 }}>{p.name}</div>
                    <div style={{ fontSize: 'var(--text-micro)', color: 'var(--text-faint)', fontFamily: 'var(--font-mono)', whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>
                      {p.repo_path || '仓库路径未配置'}{p.archived_at ? ` · 归档于 ${fmtFull(p.archived_at)}` : ''}
                    </div>
                  </div>
                  <button className="btn btn-sm" disabled={recycleBusy} onClick={() => doRestore(p)} title="恢复到在用项目列表">
                    <Icon name="refresh" size={13} />恢复
                  </button>
                  <button className="btn btn-sm" style={{ color: 'var(--red)' }} disabled={recycleBusy} onClick={() => setPurgeTarget(p)} title="彻底删除，不可恢复">
                    <Icon name="trash" size={13} />彻底删除
                  </button>
                </div>
              ))}
            </div>
          </div>
        </div>
      )}
      {purgeTarget && (
        <ConfirmModal
          msg={`彻底删除项目「${purgeTarget.name}」？`}
          sub="将级联清除该项目的需求、变更请求、审核记录、预览环境、测试记录和规格索引，不可恢复。仓库内 .autoforge/ 文件不受影响。"
          okLabel="彻底删除"
          danger
          onOk={doPurge}
          onCancel={() => setPurgeTarget(null)}
        />
      )}
    </div>
  );
}
