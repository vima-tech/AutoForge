-- Mark requirements that re-entered the pipeline via "恢复需求" (restore after a
-- revert), so the UI can flag them with a small dot in the queue. 0 = normal,
-- 1 = restored from a reverted state. Persistent: survives the re-merge so the
-- requirement's restored origin stays visible.
ALTER TABLE issues ADD COLUMN restored_from_revert INTEGER NOT NULL DEFAULT 0;
