# Ondas development

Ondas is a read-only Rust library for backend-independent waveform analysis.

Before any work, run `tree docs`. Use the filenames and tree structure to identify and read the documents relevant to the task. Apply `how-to-document.md` when working on `docs/` or comments.

Public API contracts and usage examples belong in Rustdoc in `src/`.

Target Simplified Technical English in `docs/` and public Rustdoc.

Keep Git, editors, agents, credentials and signing on the host. Run project commands through `./dev`. Follow `development-workflow.md` for hooks, required checks and release policy.

Keep materialized fixtures, vendor files, credentials and user-environment details out of Git and Docker build contexts. Use ignored local configuration and logs.
