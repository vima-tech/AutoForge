import React, { useEffect, useRef, useState } from 'react';
import Icon from './Icon';
import {
  importIssueAttachment, listIssueAttachments, issueAttachmentDataUrl,
  openIssueAttachment, deleteIssueAttachment, type IssueAttachment,
} from '../services';

// 后端附件白名单（attachments_common.rs）：图片 + 只读文档。
const ACCEPT = '.png,.jpg,.jpeg,.webp,.gif,.pdf,.txt,.log,.md,.csv,.json,.yaml,.yml,.toml';
const MAX_BYTES = 10 * 1024 * 1024;

/** 把 File 读成后端 import 载荷（base64 去掉 data: 前缀）。 */
export async function fileToUpload(file: File): Promise<{ file_name: string; mime_hint: string; data_base64: string }> {
  const buf = await file.arrayBuffer();
  let binary = '';
  const bytes = new Uint8Array(buf);
  for (let i = 0; i < bytes.length; i++) binary += String.fromCharCode(bytes[i]);
  return { file_name: file.name, mime_hint: file.type || '', data_base64: btoa(binary) };
}

const isImage = (name: string, type?: string) =>
  (type ?? '').startsWith('image/') || /\.(png|jpe?g|webp|gif)$/i.test(name);

/**
 * 需求附件条。两种模式：
 * - 暂存模式（issueId 为空）：受控持有待提交 File[]，提交前不落库；父级提交后用 fileToUpload 逐个上传。
 * - 在线模式（issueId 有值）：选文件即上传到该需求，列出/预览/删除直接读后端。
 * 样式只用 index.css 变量与既有类（.chip/.btn-sm/<Icon>）。
 */
export default function AttachmentBar({
  issueId, staged, onStaged, compact,
}: {
  issueId?: string | null;
  staged?: File[];
  onStaged?: (files: File[]) => void;
  compact?: boolean;
}) {
  const live = !!issueId;
  const fileRef = useRef<HTMLInputElement | null>(null);
  const [items, setItems] = useState<IssueAttachment[]>([]);
  const [thumbs, setThumbs] = useState<Record<string, string>>({});
  const [stagedThumbs, setStagedThumbs] = useState<Record<string, string>>({});
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState('');

  const refresh = async () => {
    if (!issueId) return;
    try {
      const list = await listIssueAttachments(issueId);
      setItems(list);
      // 图片缩略图：逐个取 data url（≤10MB）。
      const next: Record<string, string> = {};
      await Promise.all(list.filter(a => a.kind === 'image').map(async a => {
        try { next[a.id] = await issueAttachmentDataUrl(a.id); } catch { /* skip */ }
      }));
      setThumbs(next);
    } catch (e) { setErr(String(e)); }
  };

  useEffect(() => { void refresh(); /* eslint-disable-next-line react-hooks/exhaustive-deps */ }, [issueId]);

  // 暂存图片本地预览（objectURL），卸载时回收。
  useEffect(() => {
    if (live || !staged) return;
    const map: Record<string, string> = {};
    staged.forEach((f, i) => { if (isImage(f.name, f.type)) map[`${i}:${f.name}`] = URL.createObjectURL(f); });
    setStagedThumbs(map);
    return () => { Object.values(map).forEach(u => URL.revokeObjectURL(u)); };
  }, [staged, live]);

  const pick = () => fileRef.current?.click();

  const onPick = async (e: React.ChangeEvent<HTMLInputElement>) => {
    const files = Array.from(e.target.files ?? []);
    e.target.value = ''; // 允许再次选同名文件
    if (!files.length) return;
    setErr('');
    const tooBig = files.find(f => f.size > MAX_BYTES);
    if (tooBig) { setErr(`「${tooBig.name}」超过 10 MB 上限`); return; }

    if (!live) {
      onStaged?.([...(staged ?? []), ...files]);
      return;
    }
    setBusy(true);
    try {
      for (const f of files) {
        const payload = await fileToUpload(f);
        await importIssueAttachment({ issue_id: issueId!, ...payload });
      }
      await refresh();
    } catch (e) { setErr(String(e)); }
    finally { setBusy(false); }
  };

  const removeStaged = (idx: number) => onStaged?.((staged ?? []).filter((_, i) => i !== idx));
  const removeLive = async (id: string) => {
    setBusy(true);
    try { await deleteIssueAttachment(id); await refresh(); }
    catch (e) { setErr(String(e)); }
    finally { setBusy(false); }
  };

  const chipStyle: React.CSSProperties = {
    display: 'inline-flex', alignItems: 'center', gap: 6, maxWidth: 200,
    background: 'var(--bg-3)', border: '1px solid var(--border-strong)',
    borderRadius: 8, padding: '3px 6px 3px 4px', fontSize: 'var(--text-label)', color: 'var(--text-2)',
  };
  const thumbStyle: React.CSSProperties = { width: 24, height: 24, borderRadius: 5, objectFit: 'cover', flexShrink: 0 };
  const nameStyle: React.CSSProperties = { overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' };

  return (
    <div style={{ display: 'flex', flexWrap: 'wrap', alignItems: 'center', gap: 8 }}>
      <input ref={fileRef} type="file" multiple accept={ACCEPT} onChange={onPick} style={{ display: 'none' }} />
      <button type="button" className="btn btn-sm btn-ghost" onClick={pick} disabled={busy}
        title="添加图片或附件（PNG/JPG/WebP/GIF/PDF/TXT/MD/JSON/CSV/YAML/TOML，≤10MB）">
        <Icon name="paperclip" size={13} />{compact ? '' : '添加图片/附件'}
      </button>

      {/* 暂存模式 chips */}
      {!live && (staged ?? []).map((f, i) => {
        const key = `${i}:${f.name}`;
        return (
          <span key={key} style={chipStyle}>
            {stagedThumbs[key]
              ? <img src={stagedThumbs[key]} alt="" style={thumbStyle} />
              : <Icon name="file" size={14} style={{ flexShrink: 0, color: 'var(--text-3)' }} />}
            <span style={nameStyle}>{f.name}</span>
            <button type="button" className="icon-btn" style={{ width: 18, height: 18 }}
              onClick={() => removeStaged(i)} title="移除"><Icon name="x" size={11} /></button>
          </span>
        );
      })}

      {/* 在线模式 chips */}
      {live && items.map(a => (
        <span key={a.id} style={chipStyle}>
          {a.kind === 'image' && thumbs[a.id]
            ? <img src={thumbs[a.id]} alt="" style={{ ...thumbStyle, cursor: 'pointer' }}
                onClick={() => void openIssueAttachment(a.id)} />
            : <Icon name="file" size={14} style={{ flexShrink: 0, color: 'var(--text-3)' }} />}
          <span style={{ ...nameStyle, cursor: 'pointer' }} onClick={() => void openIssueAttachment(a.id)}
            title="打开">{a.original_name}</span>
          <button type="button" className="icon-btn" style={{ width: 18, height: 18 }}
            onClick={() => void removeLive(a.id)} title="删除"><Icon name="trash" size={11} /></button>
        </span>
      ))}

      {err && <span style={{ fontSize: 'var(--text-label)', color: 'var(--red)' }}>{err}</span>}
    </div>
  );
}
