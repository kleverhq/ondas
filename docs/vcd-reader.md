# VCD validation and replay

`src/backends/vcd.rs` owns parsing and replay. `docs/images/vcd-native-flow.drawio.svg` shows the serial uncached path; the parallel paths use the same completed-tick engine.

## Opening settles more than syntax

Opening validates the whole body before publishing handles. This also settles storage: real declarations carrying only string records become string signals, including numeric-looking text. Mixed real/string histories fail. A header-only hierarchy would expose handles with an encoding that later records could invalidate.

Value digits are validated before selection filtering or truncation. Discarding high bits of an over-width value does not make invalid source digits acceptable. Querying a narrow subset therefore does not imply that malformed unselected text was ignored at open.

Large Unix files can validate independent timestamp-bounded chunks and summarize selected prefixes in parallel. Uncertain boundaries fall back to serial work. Full-range queries can also buffer bounded batches of selected records before ordered delivery. These are transient parsing buffers, not retained histories or a seek index; file and byte inputs can have different costs.

## A checkpoint needs parser context

The identifier map cannot seek to a tick. Without reusable state, a late read must establish selected state from the body. A selection can retain its last completed state and one bounded boundary checkpoint. The checkpoint must include dump-block context, not just a byte offset and timestamp.

A query context may need events at the absolute preceding tick, so session reuse requires a position before the session start. Reusing a snapshot after that boundary would lose those counts. Earlier requests can use the boundary checkpoint or replay; stopped and failed traversals discard reusable state. Cache and parallel-buffer limits are defined in the source, not a separate documentation budget.

## Compatibility choices

Ordinary event records count even at the first tick. Records inside dump blocks are snapshots instead of occurrences. Removing every first-tick event would lose genuine observations; counting every checkpoint would invent triggers. Mixed checkpoint/event ticks cannot establish physical multiplicity. A resume record likewise does not identify a hidden transition time.

Declaration merging uses source spelling and parent identity. Equal logical paths are insufficient. Attached range-like text is interpreted only under the parser's width and escaping rules; an escaped identifier can contain literal brackets. Public lookup must not reapply those syntax guesses.

The parser accepts integer-valued decimal timestamps and the Migen closed declaration list that enters dumpvars without enddefinitions. These are compatibility choices, not general EVCD support. Supported syntax and rejection categories are documented in crate Rustdoc; conformance evidence is interpreted in `fixture-conformance.md`.
