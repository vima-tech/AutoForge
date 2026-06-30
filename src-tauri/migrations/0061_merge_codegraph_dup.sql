-- 对账：0060 的 code_intel 种子可能与用户「已手动添加的 codegraph」重复，导致 MCP 列表里
-- 出现两行 codegraph。这里把它们合并成一行——一个 MCP server 本就能同时担任「Agent 工具源」
-- （按 agent_ids_json 勾选，pull）与「代码情报提供者」（role=code_intel，push），二者在后端是
-- 独立查询，无需各占一行。
--
-- 策略：若存在另一条 command=codegraph、尚无 role 的用户行，则把种子的 role+能力映射并入它，
-- 再删掉种子。保留用户原行（含其 Agent 勾选与命名），仅移除自动生成的种子，零用户数据丢失。
-- 幂等：无重复时（全新安装只有种子）两条语句都不命中，种子原样保留。

-- 1) 把种子的 role + 能力映射并入用户已有的 codegraph 行（仅当其还没设 role）。
UPDATE mcp_servers
SET role = 'code_intel',
    capability_map_json = (
      SELECT capability_map_json FROM mcp_servers WHERE id = 'code-intel-codegraph'
    )
WHERE id <> 'code-intel-codegraph'
  AND lower(command) = 'codegraph'
  AND (role IS NULL OR role = '')
  AND EXISTS (SELECT 1 FROM mcp_servers WHERE id = 'code-intel-codegraph');

-- 2) 用户行已升级为 code_intel 后，删除重复的种子行。
DELETE FROM mcp_servers
WHERE id = 'code-intel-codegraph'
  AND EXISTS (
    SELECT 1 FROM mcp_servers
    WHERE id <> 'code-intel-codegraph'
      AND lower(command) = 'codegraph'
      AND role = 'code_intel'
  );
