CREATE INDEX tetcore_logs_created_at_msg_idx ON tetcore_logs USING BTREE (created_at, (logs->>'msg'));