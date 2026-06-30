-- 精简配置：代码情报能力映射改为「可选，留空走约定」。约定发现能可靠识别 codegraph
-- （工具名含 search→定位、含 caller→调用者；参数名 query/symbol + projectPath 自动匹配），
-- 故把**自动种子的**能力映射清空，让 codegraph 默认零配置走约定，「高级」面板留空。
--
-- 仅清空与 0060 种子**逐字一致**的映射（即从未被用户手改过的自动值），用户自定义的映射不动。
UPDATE mcp_servers
SET capability_map_json = '{}'
WHERE role = 'code_intel'
  AND capability_map_json = '{"locate_symbol":{"tool":"codegraph_search","args":{"query":"$SYMBOL","projectPath":"$REPO","limit":1}},"find_callers":{"tool":"codegraph_callers","args":{"symbol":"$SYMBOL","projectPath":"$REPO","limit":5}}}';
