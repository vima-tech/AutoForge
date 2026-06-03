INSERT OR IGNORE INTO messages (id, conversation_id, from_agent, content_json, created_at) VALUES
  ('msg-attach-image-001','conv-g1',NULL,
   '[{"t":"image","label":"assistant-page.png","meta":"PNG · 248 KB · 1440×900","color":"#4f8ed1"}]',
   datetime('now','-119 minutes')),
  ('msg-attach-file-001','conv-g1',NULL,
   '[{"t":"file","name":"feedback-raw.json","meta":"JSON · 3.2 KB","color":"#e0a32e"}]',
   datetime('now','-118 minutes'));
