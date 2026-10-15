-- Retention and prune deletes run inside SQLite's single write lock. Without
-- these indexes each pass read its whole table (the webhook payloads alone are
-- about 180 MB) while holding the lock: seconds once the page cache was cold,
-- long enough for agent requests to time out with `database is locked`.
CREATE INDEX webhook_deliveries_received_idx ON webhook_deliveries(received_at);
CREATE INDEX capacity_samples_recorded_idx ON capacity_samples(recorded_at);
CREATE INDEX fleet_agent_logs_host_received_idx ON fleet_agent_logs(host_id,received_at);
