-- Last-run record for supervised background jobs: the ops
-- status panel reads when the payment sweep last completed
-- a pass. One row per job, upserted by the job itself. This
-- is observability data, never a lock or a claim.
CREATE TABLE ops_job_runs (
    job TEXT PRIMARY KEY,
    last_run_at TIMESTAMPTZ NOT NULL,
    last_result TEXT NOT NULL DEFAULT ''
);
