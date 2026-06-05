-- Material files represent one DB row per physical file path in a project.
-- Older versions could leave duplicate rows for the same rel_path after sync
-- or AI-driven moves. Keep the most recently touched row, then enforce it.

DELETE FROM material_files
WHERE id NOT IN (
  SELECT id
  FROM (
    SELECT id
    FROM material_files mf
    WHERE mf.rel_path = material_files.rel_path
      AND mf.project_id = material_files.project_id
    ORDER BY updated_at DESC, created_at DESC, id DESC
    LIMIT 1
  )
);

CREATE UNIQUE INDEX IF NOT EXISTS ux_material_files_project_rel_path
  ON material_files(project_id, rel_path);
