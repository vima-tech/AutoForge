-- 物料库文件夹去重 + 唯一约束。
--
-- 背景：material_folders 无 (project_id, parent_id, name) 唯一约束，而
-- get_or_create_material_folder 是非原子的 check-then-insert。多个 IPC 命令
-- （list_material_folders / list_material_files / search_material_files）入口
-- 都会触发 ensure_and_sync_docs → sync_repo_docs 扫描 .autoforge/docs。当它们
-- 并发执行且某子目录还没有 DB 记录时，两个同步流程的 SELECT 都查空、各 INSERT
-- 一条，于是同一磁盘目录在 DB 里有了两条记录。此后 sync_deleted_folders 只按
-- 「磁盘目录是否存在」清理，两条都指向存在的目录故永久残留，物料库里就显示重复
-- 文件夹（一条挂着文件、另一条为空幽灵）。
--
-- 本迁移：1) 合并历史重复记录（把文件/子文件夹挂到存活记录、删掉多余记录）；
--         2) 建表达式唯一索引（COALESCE(parent_id,'') 让 NULL 父级也参与去重，
--            因为 SQLite 的 UNIQUE 把多个 NULL 视为互不相同）。
-- 配套 Rust 改动把 INSERT 改为幂等的 INSERT OR IGNORE + 回查，根除竞态。

-- 1) 为每个 (project_id, parent_id, name) 分组选出存活记录：优先保留挂文件多的，
--    再按 created_at 早、id 小兜底。其余为待合并的重复记录。
CREATE TEMP TABLE _mf_rank AS
SELECT
  f.id,
  f.project_id,
  COALESCE(f.parent_id, '') AS pkey,
  f.name,
  ROW_NUMBER() OVER (
    PARTITION BY f.project_id, COALESCE(f.parent_id, ''), f.name
    ORDER BY
      (SELECT COUNT(*) FROM material_files mf WHERE mf.folder_id = f.id) DESC,
      f.created_at ASC,
      f.id ASC
  ) AS rn
FROM material_folders f;

-- 重复记录 → 存活记录 的映射。
CREATE TEMP TABLE _mf_remap AS
SELECT d.id AS dup_id, s.id AS keep_id
FROM _mf_rank d
JOIN _mf_rank s
  ON s.project_id = d.project_id
 AND s.pkey = d.pkey
 AND s.name = d.name
 AND s.rn = 1
WHERE d.rn > 1;

-- 2) 把重复文件夹下的文件改挂到存活记录。
UPDATE material_files
SET folder_id = (SELECT keep_id FROM _mf_remap WHERE dup_id = material_files.folder_id)
WHERE folder_id IN (SELECT dup_id FROM _mf_remap);

-- 3) 把以重复文件夹为父的子文件夹改挂到存活记录。
UPDATE material_folders
SET parent_id = (SELECT keep_id FROM _mf_remap WHERE dup_id = material_folders.parent_id)
WHERE parent_id IN (SELECT dup_id FROM _mf_remap);

-- 4) 删除重复记录。
DELETE FROM material_folders WHERE id IN (SELECT dup_id FROM _mf_remap);

DROP TABLE _mf_rank;
DROP TABLE _mf_remap;

-- 5) 唯一索引：表达式索引让 parent_id NULL（顶层文件夹）也能正确去重。
CREATE UNIQUE INDEX IF NOT EXISTS idx_material_folders_uniq
  ON material_folders(project_id, COALESCE(parent_id, ''), name);
