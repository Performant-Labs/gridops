-- Every reconciler pass finishes the ended fleet placements that still have
-- something to settle (#259). A failed placement is unsettled while its
-- absence is unrecorded or it still holds capacity. Without these indexes,
-- finding those meant visiting every failure in the fleet's history on every
-- pass. Both indexes hold only the rows still to settle.
CREATE INDEX workload_placements_unsettled_failure_idx ON workload_placements(created_at)
  WHERE state='failed' AND (local_absence_proven_at IS NULL OR delayed_starts_excluded_at IS NULL);
CREATE INDEX capacity_allocations_held_idx ON capacity_allocations(placement_id)
  WHERE state!='released';
