# Benchmark boundaries and comparable evidence

Workload definitions, parameters and group IDs live in `benches/`; commands live in `justfile`. Keep Criterion's runner and local baselines. Timing thresholds are not correctness or release gates.

## Name the work being measured

A fresh waveform can use a warm filesystem cache. A fresh selection includes grouping and state establishment; a prepared selection can retain state; a primed repeat can measure a cache hit with no native traversal. These are different experiments. Include result destruction and query cleanup, including after a stop. For byte input, keep file loading outside opening unless it is explicitly part of the question.

A callback limit does not identify how much decoding or loading preceded delivery. Selective payload access does not prove skipped decoding. Count delivered records, native advances, allocations and SDK residency separately when they matter. A duration alone does not establish any of those quantities. Common-engine counters exclude reader and SDK work.

The setup boundary must match across revisions and competitors. A GHW opening pass validates the body and can build a payload-position index, while another reader may defer history loading until its first query. Report opening and first access together when comparing their total cost. A retained-history lookup is not comparable to that first access.

Source type names do not prove storage costs. For example, a source called shortreal can contain binary64 records. Reader qualification and actual storage, rather than a declaration label, determine whether a conversion path was exercised. The reader documents explain prefix work, decompression, seeks, collection and worker communication.

## Control the source of a difference

Compare the same artifact, explicit reader, selection, bounds and consumption policy. Vary history length, unrelated activity, projection width, setup or output volume only when that variable answers the question. Aliases, slices and repeated positions can share base reads; report unique bases as well as output slots.

A large file or late end tick does not establish selected activity. Use controlled histories alongside recordings. Cross-format comparisons require equivalent selected observations, not equal file sizes or physical durations. The matched-clock sources share activity in tick positions while their timescales and surrounding designs differ.

Check independent expected results and workload assumptions before timing, including filtered runs. Do not put performance expectations in fixture sidecars or use unsupported values replaced with placeholders. Bound retained traces unless materialization is the work under study.

Record source and fixture commits, SDK identity, loaded runtime, workload IDs and setup boundary in ignored evidence. Reuse a baseline only with matching IDs and inputs. Keep SDK sweep order stable because output directories use list positions. Separate worktrees do not share Criterion output automatically.

Repeat an unchanged revision to estimate drift before attributing a Criterion regression label to code. Compilation and Criterion `--test` establish execution, not speed. A newer-file FSDB skip omits the affected groups and supplies no timing evidence; inspect skip reporting rather than accepting a successful command as a complete comparison.
