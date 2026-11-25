-- The ended fleet placements the reconciler still has to finish (#259).
--
-- Every reconciler pass used to read and finish every placement that had
-- ever ended, nearly every placement ever made, so each pass's work and the
-- reconciler's memory grew with the fleet's history until it was killed at
-- its memory limit. A placement now joins this queue when it ends (`failed`,
-- `cleaned` or `uncertain`): the reconciler finishes it, and takes a failed
-- or cleaned one off once nothing is left to do, so a pass reads the queue
-- and none of the history. An uncertain placement stays until it is cleaned.
--
-- Whether a placement is finished at all rests on the triggers below. A later
-- migration that rebuilds `workload_placements` or `runners` (create, copy,
-- drop, rename) drops them without an error, and dropping the old
-- `workload_placements` cascades to this queue: it must create them again and
-- queue what was queued. `db::tests::fresh_database_applies_every_migration`
-- checks they exist.
CREATE TABLE placement_finish_queue (
  placement_id TEXT PRIMARY KEY NOT NULL REFERENCES workload_placements(id) ON DELETE CASCADE
) STRICT;

CREATE TRIGGER workload_placements_queue_finish_on_insert
AFTER INSERT ON workload_placements
WHEN NEW.state IN ('failed','cleaned','uncertain')
BEGIN
  INSERT OR IGNORE INTO placement_finish_queue (placement_id) VALUES (NEW.id);
END;

CREATE TRIGGER workload_placements_queue_finish_on_end
AFTER UPDATE OF state ON workload_placements
WHEN NEW.state IN ('failed','cleaned','uncertain') AND NEW.state IS NOT OLD.state
BEGIN
  INSERT OR IGNORE INTO placement_finish_queue (placement_id) VALUES (NEW.id);
END;

-- A queued placement that leaves the ended states (an uncertain one found
-- running again) has nothing to finish until it ends again.
CREATE TRIGGER workload_placements_unqueue_finish_on_resume
AFTER UPDATE OF state ON workload_placements
WHEN NEW.state NOT IN ('failed','cleaned','uncertain')
BEGIN
  DELETE FROM placement_finish_queue WHERE placement_id = NEW.id;
END;

-- A runner row that comes back into service after its placement was finished
-- (a late job event marks a retired runner busy or online by name) has its
-- ended placement finished again, which retires it again.
CREATE TRIGGER runners_queue_finish_on_return
AFTER UPDATE OF status ON runners
WHEN OLD.status IN ('deleted','failed') AND NEW.status NOT IN ('deleted','failed')
BEGIN
  INSERT OR IGNORE INTO placement_finish_queue (placement_id)
  SELECT id FROM workload_placements
   WHERE workload_id = NEW.id AND state IN ('failed','cleaned','uncertain');
END;

-- Placements that ended before this migration are queued by the reconciler,
-- only when they still have something to finish, in short batches by rowid
-- (`placement::queue_earlier_ended_placements`): up to `end_rowid`, the last
-- placement that existed here. Later ones are queued by the triggers above.
-- Nothing is written to the existing placements, so this migration takes no
-- longer on a large table than on an empty one.
CREATE TABLE placement_finish_sweep (
  singleton INTEGER PRIMARY KEY NOT NULL CHECK (singleton = 1),
  next_rowid INTEGER NOT NULL,
  end_rowid INTEGER NOT NULL
) STRICT;
INSERT INTO placement_finish_sweep (singleton,next_rowid,end_rowid)
SELECT 1,0,COALESCE(MAX(rowid),0) FROM workload_placements;
