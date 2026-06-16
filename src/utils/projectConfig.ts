/**
 * Project run-config parse/build helpers.
 *
 * The per-project operational config (preview / test / scan / deploy / data-masking)
 * is persisted as JSON in the repo's `.autoforge/run-config.json` (source of truth);
 * the DB column is a fallback for not-yet-migrated projects. This module is the
 * 运行配置 tab's only editor source: `parseProjectConfigForm` (JSON, with a legacy
 * YAML fallback) and `buildProjectConfig` (emits JSON). Schema mirrors the Rust
 * structs in `tasks/testing.rs`, `commands/deploy.rs`, `commands/cr_preview.rs`,
 * `commands/dev_server.rs` and `core/mask.rs` (JSON is valid YAML, so those
 * `serde_yaml` consumers parse it unchanged).
 */

export type MaskRule = 'mask' | 'hash' | 'drop';

export type SensitiveFieldRow = { table: string; fields: string; rule: MaskRule };

export type ProjectConfigForm = {
  // dev — 审计预览环境 (cr_preview.rs / dev_server.rs)。预览地址固定为
  // http://localhost:{port}，不再可配；prod URL 也已移除。
  devKind: 'web' | 'tauri';
  devCommand: string;
  appCommand: string;
  // test / quality — 巡检与合并前测试 (testing.rs)
  testUnit: string;
  testUnitTimeout: string;
  testIntegration: string;
  testIntegrationTimeout: string;
  qualityLint: string;
  qualityTyping: string;
  qualitySecurity: string;
  // project / preview / deploy — 部署脚本生成 (deploy.rs)
  projectLanguage: string;
  projectFramework: string;
  previewBuild: string;
  previewStart: string;
  deployCommand: string;
  // preview.sensitive_fields — 预览数据脱敏 (mask.rs)
  sensitiveFields: SensitiveFieldRow[];
};

export const EMPTY_PROJECT_CONFIG: ProjectConfigForm = {
  devKind: 'web', devCommand: '', appCommand: '',
  testUnit: '', testUnitTimeout: '', testIntegration: '', testIntegrationTimeout: '',
  qualityLint: '', qualityTyping: '', qualitySecurity: '',
  projectLanguage: '', projectFramework: '', previewBuild: '', previewStart: '', deployCommand: '',
  sensitiveFields: [],
};

// Top-level sections this editor owns; anything else is preserved verbatim.
const MANAGED_KEYS = new Set(['dev', 'prod', 'test', 'quality', 'preview', 'deploy', 'project']);

const empty = (): ProjectConfigForm => ({ ...EMPTY_PROJECT_CONFIG, sensitiveFields: [] });

// ── parse ──────────────────────────────────────────────────────────────────────

export function parseProjectConfigForm(raw: string | null): ProjectConfigForm {
  if (!raw || !raw.trim()) return empty();
  if (raw.trim().startsWith('{')) {
    try { return parseJsonForm(JSON.parse(raw)); } catch { /* fall through to legacy YAML */ }
  }
  return parseYamlForm(raw);
}

/* eslint-disable @typescript-eslint/no-explicit-any */
function str(v: any): string {
  return typeof v === 'string' ? v : v == null ? '' : String(v);
}

function parseJsonForm(obj: any): ProjectConfigForm {
  if (!obj || typeof obj !== 'object') return empty();
  const dev = obj.dev ?? {};
  const test = obj.test ?? {};
  const quality = obj.quality ?? {};
  const project = obj.project ?? {};
  const preview = obj.preview ?? {};
  const deploy = obj.deploy ?? {};
  const sens: any[] = Array.isArray(preview.sensitive_fields) ? preview.sensitive_fields : [];
  return {
    devKind: dev.kind === 'tauri' ? 'tauri' : 'web',
    devCommand: str(dev.command),
    appCommand: str(dev.app_command),
    testUnit: str(test.unit?.command),
    testUnitTimeout: test.unit?.timeout != null ? String(test.unit.timeout) : '',
    testIntegration: str(test.integration?.command),
    testIntegrationTimeout: test.integration?.timeout != null ? String(test.integration.timeout) : '',
    qualityLint: str(quality.lint),
    qualityTyping: str(quality.typing),
    qualitySecurity: str(quality.security),
    projectLanguage: str(project.language),
    projectFramework: str(project.framework),
    previewBuild: str(preview.build?.command),
    previewStart: str(preview.start?.command),
    deployCommand: str(deploy.command),
    sensitiveFields: sens.map(r => ({
      table: str(r?.table),
      fields: Array.isArray(r?.fields) ? r.fields.map(str).join(', ') : str(r?.fields),
      rule: (r?.rule === 'hash' || r?.rule === 'drop' ? r.rule : 'mask') as MaskRule,
    })).filter(r => r.table),
  };
}
/* eslint-enable @typescript-eslint/no-explicit-any */

// ── legacy YAML parse (for un-migrated DB columns) ──────────────────────────────

function unquote(v: string): string {
  const t = v.trim();
  if (t.length >= 2 && ((t[0] === '"' && t.endsWith('"')) || (t[0] === "'" && t.endsWith("'")))) {
    return t.slice(1, -1);
  }
  return t;
}

function flattenScalars(yaml: string): Record<string, string> {
  const out: Record<string, string> = {};
  const stack: { indent: number; key: string }[] = [];
  for (const raw of yaml.split('\n')) {
    const trimmed = raw.trim();
    if (!trimmed || trimmed.startsWith('#') || trimmed.startsWith('-')) continue;
    const m = trimmed.match(/^([A-Za-z_][\w]*):\s*(.*)$/);
    if (!m) continue;
    const indent = raw.length - raw.trimStart().length;
    while (stack.length && stack[stack.length - 1].indent >= indent) stack.pop();
    const path = [...stack.map(s => s.key), m[1]].join('.');
    if (m[2] === '') stack.push({ indent, key: m[1] });
    else out[path] = unquote(m[2]);
  }
  return out;
}

function parseFlowList(v: string): string[] {
  const t = v.trim();
  if (t.startsWith('[') && t.endsWith(']')) {
    return t.slice(1, -1).split(',').map(unquote).filter(Boolean);
  }
  return t ? [unquote(t)] : [];
}

function parseSensitive(yaml: string): SensitiveFieldRow[] {
  const lines = yaml.split('\n');
  let i = lines.findIndex(l => /^\s*sensitive_fields:\s*$/.test(l));
  if (i < 0) return [];
  const headerIndent = lines[i].length - lines[i].trimStart().length;
  const rows: SensitiveFieldRow[] = [];
  let cur: SensitiveFieldRow | null = null;
  const applyKV = (row: SensitiveFieldRow, kv: string) => {
    const m = kv.match(/^([A-Za-z_]\w*):\s*(.*)$/);
    if (!m) return;
    if (m[1] === 'table') row.table = unquote(m[2]);
    else if (m[1] === 'rule') row.rule = (unquote(m[2]) as MaskRule) || 'mask';
    else if (m[1] === 'fields') row.fields = parseFlowList(m[2]).join(', ');
  };
  for (i += 1; i < lines.length; i++) {
    const raw = lines[i];
    const trimmed = raw.trim();
    if (!trimmed) continue;
    const indent = raw.length - raw.trimStart().length;
    if (indent <= headerIndent) break;
    const dash = trimmed.match(/^-\s*(.*)$/);
    if (dash) {
      if (cur) rows.push(cur);
      cur = { table: '', fields: '', rule: 'mask' };
      if (dash[1]) applyKV(cur, dash[1]);
    } else if (cur) {
      applyKV(cur, trimmed);
    }
  }
  if (cur) rows.push(cur);
  return rows.filter(r => r.table);
}

function parseYamlForm(yaml: string): ProjectConfigForm {
  const s = flattenScalars(yaml);
  return {
    devKind: s['dev.kind'] === 'tauri' ? 'tauri' : 'web',
    devCommand: s['dev.command'] ?? '',
    appCommand: s['dev.app_command'] ?? '',
    testUnit: s['test.unit.command'] ?? '',
    testUnitTimeout: s['test.unit.timeout'] ?? '',
    testIntegration: s['test.integration.command'] ?? '',
    testIntegrationTimeout: s['test.integration.timeout'] ?? '',
    qualityLint: s['quality.lint'] ?? '',
    qualityTyping: s['quality.typing'] ?? '',
    qualitySecurity: s['quality.security'] ?? '',
    projectLanguage: s['project.language'] ?? '',
    projectFramework: s['project.framework'] ?? '',
    previewBuild: s['preview.build.command'] ?? '',
    previewStart: s['preview.start.command'] ?? '',
    deployCommand: s['deploy.command'] ?? '',
    sensitiveFields: parseSensitive(yaml),
  };
}

// ── build (JSON, persisted to .autoforge/run-config.json) ───────────────────────

export function buildProjectConfig(form: ProjectConfigForm, existing: string | null): string | undefined {
  const obj: Record<string, unknown> = {};

  const dev: Record<string, unknown> = {};
  if (form.devKind === 'tauri') dev.kind = 'tauri';
  if (form.devCommand.trim()) dev.command = form.devCommand.trim();
  if (form.devKind === 'tauri' && form.appCommand.trim()) dev.app_command = form.appCommand.trim();
  if (Object.keys(dev).length) obj.dev = dev;

  const test: Record<string, unknown> = {};
  if (form.testUnit.trim()) {
    const u: Record<string, unknown> = { command: form.testUnit.trim() };
    const t = parseInt(form.testUnitTimeout, 10);
    if (Number.isFinite(t) && t > 0) u.timeout = t;
    test.unit = u;
  }
  if (form.testIntegration.trim()) {
    const it: Record<string, unknown> = { command: form.testIntegration.trim() };
    const t = parseInt(form.testIntegrationTimeout, 10);
    if (Number.isFinite(t) && t > 0) it.timeout = t;
    test.integration = it;
  }
  if (Object.keys(test).length) obj.test = test;

  const quality: Record<string, unknown> = {};
  if (form.qualityLint.trim()) quality.lint = form.qualityLint.trim();
  if (form.qualityTyping.trim()) quality.typing = form.qualityTyping.trim();
  if (form.qualitySecurity.trim()) quality.security = form.qualitySecurity.trim();
  if (Object.keys(quality).length) obj.quality = quality;

  const project: Record<string, unknown> = {};
  if (form.projectLanguage.trim()) project.language = form.projectLanguage.trim();
  if (form.projectFramework.trim()) project.framework = form.projectFramework.trim();
  if (Object.keys(project).length) obj.project = project;

  const preview: Record<string, unknown> = {};
  if (form.previewBuild.trim()) preview.build = { command: form.previewBuild.trim() };
  if (form.previewStart.trim()) preview.start = { command: form.previewStart.trim() };
  const sens = form.sensitiveFields
    .filter(r => r.table.trim())
    .map(r => ({
      table: r.table.trim(),
      fields: r.fields.split(',').map(s => s.trim()).filter(Boolean),
      rule: r.rule || 'mask',
    }));
  if (sens.length) preview.sensitive_fields = sens;
  if (Object.keys(preview).length) obj.preview = preview;

  if (form.deployCommand.trim()) obj.deploy = { command: form.deployCommand.trim() };

  // Preserve unknown top-level keys when the existing config is JSON.
  if (existing && existing.trim().startsWith('{')) {
    try {
      const ex = JSON.parse(existing);
      if (ex && typeof ex === 'object') {
        for (const k of Object.keys(ex)) {
          if (!MANAGED_KEYS.has(k) && !(k in obj)) obj[k] = ex[k];
        }
      }
    } catch { /* ignore unparseable existing */ }
  }

  return Object.keys(obj).length ? JSON.stringify(obj, null, 2) : undefined;
}
