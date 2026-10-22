# FST sections, prefix reads and decoder limits

`src/backends/fst.rs` adapts the published decoder pinned in `Cargo.toml`; it does not maintain a local decoder patch. `docs/images/fst-lib-flow.drawio.svg` separates the stateful and candidate-only paths.

## Section access is not point lookup

Opening reads geometry, the section directory and compressed hierarchy, not all value payloads. A successful open therefore does not validate every later value. The decoder's optional global time table is not requested. An outer whole-file gzip wrapper is expanded into memory even though ordinary file input is buffered.

Stateful reads filter from tick zero through the requested end. A section frame is not sufficient to establish every entering value: variable-length strings can have no initial frame value. Filtering only the requested window could turn a known state into Missing. Source-only selection reuse retains grouping, not decoded FST histories; later queries start another traversal.

Within a section, the decoder can load selected value chains before delivering their changes. It can also decompress a full frame for a small selection. `Break` cannot undo that work, and selected-slot state does not bound decompression memory. The directory and temporary offset/time tables are not a retained history cache.

Candidate-only scans can skip sections before the window because they need no entering state. They still decode and validate selected chains in the overlapping sections. Frames, redundant writes and discarded slice bits can produce extra candidates. Their validation coverage and cost differ from stateful reads without changing the candidate contract.

## Preserve uncertainty at the decoder boundary

Initial frames and event changes share a callback shape. At the first recorded tick, an event callback can be initialization or an occurrence; the adapter cannot distinguish them. Keep the observed callback rather than infer its meaning from payloads or delete all first-tick events. Blackout metadata does not reconstruct hidden circuit activity.

Enum mappings are exposed only when decoded keys match whole stored values. The adapter does not pad, reverse or infer keys from labels. The published decoder can leave label escapes undecoded; that spelling is retained. Declaration type evidence, rather than observed values or type-name guesses, determines available interpretation metadata.

Some malformed inputs and unsupported decoder cases panic in release builds. Ondas does not contain those panics. Its checks for hierarchy consistency and decoded values do not make the upstream parser hardened. An unfinished gzip-wrapper report is a decoder-state failure rather than proof of malformed data; it does not establish live-file support or promise a successful retry.
