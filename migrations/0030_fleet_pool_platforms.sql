-- A pool can run on several platforms (runtime, OS and architecture), one
-- execution profile each. A platform removed from a pool is archived rather
-- than deleted, because intents and placements still name its profile.
ALTER TABLE pool_execution_profiles ADD COLUMN archived_at INTEGER;
