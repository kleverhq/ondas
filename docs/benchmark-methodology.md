# Benchmark boundaries and comparable evidence

`benches/` defines workloads, parameters and group IDs. `justfile` defines commands. Keep Criterion's runner and local baselines. Timing thresholds are not correctness or release gates.

## Name the work being measured

A fresh waveform can use a warm filesystem cache. A fresh selection includes grouping and initial state setup. A prepared selection can retain state. A primed repeat can measure a cache hit with no native traversal. These are different experiments. Include result destruction and query cleanup, also after a stop. For byte input, keep file loading outside opening unless the experiment must measure both.

A callback limit does not show how much decoding or loading preceded delivery. Selective payload access does not prove skipped decoding. Count delivered records, native advances, allocations and SDK memory use separately when they matter. A duration alone does not establish these quantities. Common-engine counters exclude reader and SDK work.

Use the same setup boundary across revisions and competitors. GHW opening validates the body and can build an index of payload positions. Another reader can defer history loading until its first query. Compare opening plus first access when the experiment must measure their total cost. A lookup in retained history measures different work.

Source type names do not prove storage costs. For example, a source called shortreal can contain binary64 records. Check the actual storage and qualified reader to establish which conversion path ran. The reader documents explain prefix work, decompression, seeks, collection and worker communication.

## Control the source of a difference

Compare the same artifact, explicit reader, selection, bounds and consumption policy. Change one relevant variable at a time. Examples are history length, unrelated activity, projection width, setup and output volume. Aliases, slices and repeated positions can share base reads. Report unique bases as well as output slots.

A large file or late end tick does not establish selected activity. Use controlled histories alongside recordings. Cross-format comparisons need equivalent selected observations. Equal file sizes or physical durations are insufficient. The matched-clock sources share activity at the same tick positions. Their timescales and surrounding designs differ.

For OPB, readiness checks input identity and build/runtime availability without opening a waveform. Independent functional replay starts after the entire selected measurement series, including every SDK in a sweep. Replay uses the same workload, inputs and build, and compares observations with the Python mathematical model. It does not validate each timed iteration. Keep performance expectations out of fixture sidecars. Do not replace unsupported values with placeholders. Bound retained traces unless the experiment must measure materialization.

Record source and fixture commits, SDK identity, loaded runtime, workload IDs and setup boundary in ignored evidence. Reuse a baseline only with matching IDs and inputs. Keep SDK sweep order stable because output directories use list positions. Separate worktrees do not share Criterion output automatically.

Repeat an unchanged revision to estimate drift. Do this before you attribute a Criterion regression label to code. Compilation alone does not establish performance. A successful quick run qualifies only its selected case and backend configurations. Keep failed, unchecked and not-run jobs separate from passed results.

## OPB quick runner

`just opb run --backend all --quick` runs one case, `OPB-POINT-028`, on each of the seven backends. This end-to-end smoke checks readiness, Criterion measurements, saved raw data and independent verification. It uses the same workload and Criterion settings as an explicit case run. Run these recipes through `./dev`. `run` defaults to quick; `list --quick` shows its one case.

`run --full` selects all 354 implemented cases. Repeated `--case ID` arguments select a subset. The catalog's original 22 `quick_ids` are retained as source data; they do not define the current smoke selection. The `profile` target replaces the seven format-specific targets. Public CI runs the quick case separately on VCD, FST and GHW; SDK CI runs it for each selected backend and SDK. Each invocation has its own result directory and measurement-to-verification barrier.

The runner builds the `profile` target with only the selected backend feature. Each measurement process runs serially. R times the complete operation, including open, target resolution and drop. P opens a fresh waveform and resolves targets before each timed operation; result and selection destruction are timed. S also prepares the specified prior on a retained selection, then times only the next request. T prepares saved traces once and times their reads. P and S use a custom Criterion loop to keep borrowed targets local to their waveform. Saved results use workload version `opb-core-2`; historical `opb-quick-1` results retain their original semantics. A fresh waveform can still use warm operating system and SDK caches.

Verification is enabled by default. Its process count defaults to the available CPU affinity count, with `os.cpu_count()` as fallback, and is bounded by the selected jobs. `--verify-workers N` overrides it. `--no-verify` records `unchecked`. A successful replay is `passed`; it does not repair a failed measurement. Known `changed_at` values must match the model. Permitted unknown timestamps remain unknown, and their availability is counted separately. Candidates are checked as an ordered superset without retaining a candidate list.

`just opb-sweep --backend fsdb-lib --quick` uses `VERDI_HOMES` in list order; WLF, SHM and WDB use `QUESTA_HOMES`, `XCELIUM_HOMES` and `XILINX_VIVADO_HOMES`. Each entry has separate build output under `CARGO_TARGET_DIR`, or `target/` when unset. This also preserves build separation when SDK CI selects each installation through its outer wrapper. In an OPB sweep, all versions finish measurements before any verification starts. Keep the list order stable. SDK locations and loader paths belong in ignored local configuration.

Each invocation creates a fresh ignored `target/opb-runs/` directory with `run.json`, logs and native Criterion JSON. The JSON records exact case inputs, artifact identities, content hashes of source/model files, selected builds, loaded library paths, phase times and separate measurement/verification states. `ONDAS_SOURCE_REVISION`, when passed from the host, records the base Git revision; content hashes identify changes beyond that revision. Source/model or binary changes during the series prevent validation. Input files and selected SDK installations must remain unchanged. Preserve raw Criterion data after a failed check. Saved JSON can be read without a waveform, SDK or benchmark process; resolve raw file references relative to its containing run directory.

The Criterion configuration uses ten samples, 0.2 seconds of warmup and a 0.6 second measurement target for quick and explicit case runs. These durations exclude P setup and are not wall time limits. Expensive fresh opens and sessions can take much longer.

Each case/backend measurement process has a default wall limit of 600 seconds, including warmup, preparation, samples and analysis. `--timeout SECONDS` changes the limit; `--no-timeout` disables it. Verification has no timeout. The resolved limit is saved as `measurement_timeout_seconds` in `run.json`, or `null` when disabled. A timeout uses the ordinary failed-measurement path with `reason: timeout`; logs and any raw files remain available for diagnosis. The runner continues with the next measurement and verifies only successful measurements. Failed measurements have no usable Criterion estimate. Their elapsed wall time and the timeout limit must not replace a missing performance value.

## Retired benchmark workloads

OPB covers common opening, hierarchy, lookup, point and batch reads, sessions, scans, traces, queries and bit composition. The core uses file paths and bit values. It does not retain the former timings for byte input, `read_metadata`, Real/String/Event values, physical alias scaling, format-specific storage controls, selected recordings, Wellen comparisons or value formatting. Independent correctness tests and published fixtures remain. Add a focused performance case only when a concrete problem requires it.

Old Criterion IDs, input files and timing boundaries differ from OPB. Historical baselines remain historical; do not reuse them as OPB baselines.
