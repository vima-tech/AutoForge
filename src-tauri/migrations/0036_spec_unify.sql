-- 规格统一管理：让 project_specs 既能内联 DB 规格，又能登记 .autoforge/specs/ 下的自由文件，
-- 并支持三档注入（always 全文常驻 / on_demand 工具按需读 / off 不暴露）。
-- 不改旧迁移，仅新增列 + 回填，完整保留现有 5 分类全文注入行为。

-- 规格来源：'db'(内容内联 content) | 'file'(指向磁盘 rel_path)
ALTER TABLE project_specs ADD COLUMN source TEXT NOT NULL DEFAULT 'db';
-- file 源的仓库相对路径，如 .autoforge/specs/tool-system-design.md（db 源为空）
ALTER TABLE project_specs ADD COLUMN rel_path TEXT NOT NULL DEFAULT '';
-- 清单/工具里展示的简短摘要（渐进读取的「声明」）
ALTER TABLE project_specs ADD COLUMN description TEXT NOT NULL DEFAULT '';
-- 注入档位：'always' | 'on_demand' | 'off'
ALTER TABLE project_specs ADD COLUMN injection TEXT NOT NULL DEFAULT 'always';

-- 现存行天然 source='db' / injection='always'（DEFAULT 已保证），保留原 5 分类全文注入。
