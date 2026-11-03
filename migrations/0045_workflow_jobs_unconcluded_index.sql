-- The jobs finished here without a conclusion, oldest first, for the
-- reconciler's job check: when a re-run appears, an earlier attempt's open
-- job is finished with no conclusion (it is not known here), and the check
-- asks GitHub how it ended. Partial, so it holds only those jobs, and the
-- check does not scan every job ever recorded on each sync.
CREATE INDEX workflow_jobs_unconcluded_created_idx ON workflow_jobs (created_at, id)
  WHERE status='completed' AND conclusion IS NULL;
