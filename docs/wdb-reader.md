# WDB reader ABI and indexed histories

`native/wdb.cpp` uses the installed reader to import declarations and index selected stored bytes. `src/backends/wdb.rs` copies observations into the common query engine. Rustdoc in `src/lib.rs` defines feature selection and runtime setup.

## The selected installation is an ABI boundary

The adapter uses private C++ interfaces. Exported symbols alone do not establish layout compatibility. The build checks required symbols and the reader's kernel dependency. Before a native session starts, the adapter checks the loaded reader, kernel and data-directory paths against the build selection. These checks do not qualify another ABI that happens to export the same names. The build selects a complete installed `rdi` or `xv` reader/kernel pair and checks the corresponding dependency.

The executable depends on the installed shared libraries. A missing library or incompatible C++ runtime can stop process startup before Rust runs. The adapter copies no vendor library into the crate. A short or invalid container header is rejected before SDK entry; an unknown container version is unsupported. This check does not validate the complete recording. After a failed SDK open, the adapter can compare the embedded DBG version with the expected signature referenced by the selected reader. A confirmed newer DBG is unsupported. Unknown reader signatures or container framing preserve the original failure; they do not justify a version skip. Native exceptions can become Rust errors; native faults and process termination cannot.

Each waveform owns its native session. SDK calls and destruction share one adapter lock. Immutable copied declaration tables need no SDK call. Values are copied before unlocking, and user visitors run after the lock is released. A visitor can query another waveform. The lock does not coordinate unrelated SDK users in the application.

## Storage and presentation have different representations

The SDK builds byte-history indexes for selected physical extents. The adapter merges exact-resolution byte cursors and applies all bytes at one tick before conversion. A declaration's complete physical extent and bit offset select its stored value. Vendor conversion supplies the logical bits. Point reads use the same indexed bytes and decoder. The SDK typed point-value transport is not interchangeable with stored memory, especially for enum values.

The older `rdi` reader can lose final updates after an intermediate return to an earlier byte value within one tick. For this reader, the adapter requests eight SDK bit indexes per physical byte and assembles the stored bytes before conversion. Point reads and traversal use the same indexes. This requires more SDK indexes and cursors than the `xv` reader's byte path. Both paths keep histories in SDK-owned storage.

Native enum tables supply codes and labels. Ordinal VHDL enums and discrete SV enums have different conversions. A table or constant does not establish that a producer recorded later changes. Real storage also needs independent qualification: the generic type bit metric can differ from the native encoder's binary64 storage.

Logic-array dimensions are flattened in vendor bit order. A top dimension whose range does not span the complete logical vector is not exposed as a single HDL bit range. Typed enum elements and unpacked members retain separate declarations where the reader supplies them. Native integral conversion preserves the complete declared width even when the text formatter omits leading zeroes. The native carrier's signed flag does not establish HDL signedness; unknown signedness stays absent. `reader_name` preserves native spelling. Logical names remove verified SV escape syntax or VHDL extended-identifier syntax; the Verilog escape flag applies only to SV names.

## Declaration support does not establish recording support

Static one-dimensional VHDL CHARACTER arrays have qualified String histories, including ascending and descending indices. The common ASCII contract preserves embedded NUL and rejects non-ASCII values. Dynamic SystemVerilog String and named Event values have no qualified read path. Selecting these declarations returns `UnsupportedSignal`. The tested read paths did not produce usable dynamic String values. This does not establish whether those values are present in an existing recording or define a WDB format limit.

Changing unsigned 129-bit enum values are also unqualified. In a tested A-to-B-to-C sequence, native reads kept the initial A code. The cause remains unresolved between recording and reading. These histories can therefore return stale values instead of an error. Successful constant values and native enum tables do not qualify changing values. Changing 65-bit X/Z enums have separate positive coverage. Neither an altered producer nor a plain bit-vector control establishes support for existing unsigned 129-bit enum recordings.

Unrecorded supported declarations remain Missing. The native waveform-presence flag can be true for a declaration with no stored updates, so it is not used to manufacture an initial value. Unsupported per-declaration layouts do not hide other histories. The adapter identifies unsupported types before requesting value constraints: an SDK constraint query can recurse through a cyclic class graph even when that class will never be read. Missing ABI symbols, inconsistent storage extents and invalid metadata remain file-level errors.

## First access builds SDK history indexes

First access to unindexed bytes loads their SDK histories, including times after the query end. A point or early stop can therefore pay the full selected-history loading cost. The SDK retains these indexes until reader destruction and manages their memory and temporary backing file. The adapter retains current physical and logical values and active cursors, without a second copy of the histories.

Bit point reads seek indexed bytes and compare earlier projected values to establish `changed_at`. A constant projection can still require many predecessor lookups. Range reads with a normalized entering checkpoint seek to their start. Compositions whose intermediate values are positive whole copies of one source can use the same point state, including source-width-aligned slices. Each output preserves the source's Missing state and change times. Other value classes and compositions can still require a prefix traversal. Opening imports all native declarations, including declarations with no recorded history. It can therefore do substantially more metadata work than another format containing only logged declarations.

The common engine supplies completed-tick samples, slices, compositions, query callbacks and Trace normalization. A traversal guard releases native cursor state on completion, `Break`, error or a Rust visitor panic. Early stopping bounds delivery; it does not undo SDK opening, metadata import or history loading. Compare opening plus first access and reader destruction when measuring total cost. OPB distinguishes complete requests (R), fresh prepared readers (P) and readers with an explicit query history (S).
