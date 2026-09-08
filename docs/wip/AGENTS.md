# Tracked Temporary Work

This permanent breadcrumb keeps the directory tracked. Do not delete it during cleanup.

- Use `../../tmp/` for ignored scratch, logs, and disposable outputs.
- Put temporary files here only when they need to be committed, reviewed, or recovered from a branch checkout.
- Every task gets a `yymmdd-slug/` subdirectory, for example `260905-fixture-contract/`. Keep execution plans, research notes, and temporary handoff artifacts inside that task directory, not at repository root or directly in this directory.
- Task directories may live on temporary branches. Before merging into `master`, move durable conclusions to their authoritative homes and remove the temporary task directories.
- Do not delete another person's or agent's artifacts outside the cleanup scope you own.
- This is a textual contribution rule; do not add scripts, hooks, or CI checks to enforce it.
