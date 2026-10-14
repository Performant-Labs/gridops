-- An unanswered read-only inspect used to mark its placement uncertain, which
-- kept the whole host from becoming ready. Return those placements to running
-- so the next inspect settles them.
UPDATE workload_placements SET state='running',updated_at=CAST(strftime('%s','now') AS INTEGER)*1000
WHERE state='uncertain'
  AND EXISTS (SELECT 1 FROM host_commands c WHERE c.placement_id=workload_placements.id AND c.state='uncertain')
  AND NOT EXISTS (SELECT 1 FROM host_commands c WHERE c.placement_id=workload_placements.id
    AND c.state='uncertain' AND c.kind<>'inspect');
