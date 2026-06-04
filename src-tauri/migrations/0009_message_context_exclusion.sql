-- Allow individual messages to be excluded from the LLM context window.
ALTER TABLE messages ADD COLUMN excluded_from_context INTEGER NOT NULL DEFAULT 0;
