-- What GitHub last said about each job (#33, #34). A table of its own rather
-- than columns on workflow_jobs, so the upgrade leaves every existing
-- workflow_jobs row and its definition as they are.
--
-- run_attempt: GitHub's `run_attempt` for the job. A re-run keeps the run id
-- and gives every job a new id, so without it the jobs of an earlier attempt
-- whose completion was missed looked like jobs of the live attempt and stayed
-- queued or in progress for good. NULL when not known, as for jobs recorded
-- before this migration; the next poll or the reconciler's job check fills it
-- in from GitHub.
--
-- confirmed_at: when GitHub last reported the job's status, by a webhook, a
-- poll or the reconciler's job check. Writes GridOps makes on its own
-- (finishing an earlier attempt's job, cancelling a finished run's orphans)
-- leave it alone.
--
-- checked_at: when the reconciler last asked GitHub about the job, so a job
-- GitHub cannot answer for is not asked about again every pass.
CREATE TABLE workflow_job_sync (
  job_id INTEGER PRIMARY KEY NOT NULL REFERENCES workflow_jobs(id) ON DELETE CASCADE,
  run_attempt INTEGER,
  confirmed_at INTEGER,
  checked_at INTEGER
);
-- Only webhooks and the poll write an open job, so an open job's updated_at
-- is when GitHub last reported it.
INSERT INTO workflow_job_sync (job_id,confirmed_at)
  SELECT id,updated_at FROM workflow_jobs WHERE status<>'completed';
-- The open jobs, oldest first, for the queue views and the reconciler's job
-- check. Partial, so it stays as small as the queue.
CREATE INDEX workflow_jobs_open_created_idx ON workflow_jobs (created_at, id)
  WHERE status<>'completed';
