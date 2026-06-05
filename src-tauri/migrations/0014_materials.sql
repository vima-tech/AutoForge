-- Material folder hierarchy per project
CREATE TABLE IF NOT EXISTS material_folders (
  id          TEXT PRIMARY KEY,
  project_id  TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  parent_id   TEXT REFERENCES material_folders(id) ON DELETE CASCADE,
  name        TEXT NOT NULL,
  sort_order  INTEGER NOT NULL DEFAULT 0,
  created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
  updated_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_material_folders_project ON material_folders(project_id);
CREATE INDEX IF NOT EXISTS idx_material_folders_parent  ON material_folders(parent_id);

-- Material files (documents, specs, assets) stored per project
CREATE TABLE IF NOT EXISTS material_files (
  id            TEXT PRIMARY KEY,
  project_id    TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  folder_id     TEXT REFERENCES material_folders(id) ON DELETE SET NULL,
  original_name TEXT NOT NULL,
  stored_name   TEXT NOT NULL,
  rel_path      TEXT NOT NULL,
  mime          TEXT NOT NULL DEFAULT 'application/octet-stream',
  size_bytes    INTEGER NOT NULL DEFAULT 0,
  sha256        TEXT NOT NULL DEFAULT '',
  description   TEXT,
  tags          TEXT NOT NULL DEFAULT '[]',
  backup_status TEXT NOT NULL DEFAULT 'none',
  backup_url    TEXT,
  backed_up_at  TEXT,
  created_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
  updated_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_material_files_project ON material_files(project_id);
CREATE INDEX IF NOT EXISTS idx_material_files_folder  ON material_files(folder_id);

-- Singleton cloud backup configuration
CREATE TABLE IF NOT EXISTS material_backup_config (
  id          TEXT PRIMARY KEY,
  provider    TEXT NOT NULL DEFAULT 'none',
  config_json TEXT NOT NULL DEFAULT '{}',
  enabled     INTEGER NOT NULL DEFAULT 0,
  updated_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

INSERT OR IGNORE INTO material_backup_config (id, provider, config_json, enabled)
VALUES ('singleton', 'none', '{}', 0);
