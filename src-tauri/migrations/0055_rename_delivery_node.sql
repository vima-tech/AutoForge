-- 交付产物节点分类 'requirement'（需求）→ 'issue'，与「需求英文统一用 issue」一致。
UPDATE delivery_artifacts SET node='issue' WHERE node='requirement';
