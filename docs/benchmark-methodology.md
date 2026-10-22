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

Check independent expected results and workload assumptions before timing, also for filtered runs. Keep performance expectations out of fixture sidecars. Do not replace unsupported values with placeholders. Bound retained traces unless the experiment must measure materialization.

Record source and fixture commits, SDK identity, loaded runtime, workload IDs and setup boundary in ignored evidence. Reuse a baseline only with matching IDs and inputs. Keep SDK sweep order stable because output directories use list positions. Separate worktrees do not share Criterion output automatically.

Repeat an unchanged revision to estimate drift. Do this before you attribute a Criterion regression label to code. Compilation and Criterion `--test` prove execution, not speed. An FSDB skip for a newer file omits the affected groups. It supplies no timing evidence. Check skip reports before you treat a successful command as a complete comparison.
