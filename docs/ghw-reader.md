# GHW source grammar and scalar recipes

GHW has no separate normative format specification. The implementation follows the GHDL producer/reader at [revision 2c451de76b07b73fd338c91d68fd5cefcb152512](https://github.com/ghdl/ghdl/tree/2c451de76b07b73fd338c91d68fd5cefcb152512). Independent fixtures provide evidence. `src/backends/ghw/` defines grammar and supported layouts. Crate Rustdoc defines public limits.

## Source conventions are not metadata

GHW stores no time resolution. Preserve ticks without adopting the native oracle's conventional 1 fs scale. Names and labels use byte-preserving Latin-1 mapping from the string table. This does not extend the ASCII sample-string contract.

Constrained enum subsets keep their base ordinals. They do not renumber labels. Character leaves remain enums, not strings. Physical values remain stored integers. The adapter does not scale them by a guessed unit. Only qualified one-dimensional bit/std_ulogic vectors become whole signals. Other arrays keep readable element structure. Null vectors and null arrays have different declaration shapes. Neither creates invented values.

## Whole values depend on ordered scalar recipes

An ordered scalar-ID recipe defines how to assemble a source value. Exact aliases can share that recipe. Reversed and overlapping recipes remain distinct. A whole value is Missing until all source positions have observations. Do not apply the explicit concat Missing-to-X rule to an incomplete source recipe.

Equal ticks can cross snapshot and cycle sections. Indexed reads must combine all final scalar payloads for the tick before assembly. Delivering each change list separately would expose an intermediate value. It could also advance a projected change time incorrectly.

## Opening and seeking have different costs

Opening validates the whole body. Metadata-only reading uses the same path. It is not a header-only SDK operation. An optional bounded index stores scalar payload offsets and ticks, not values. Footer offsets do not control seeking. If index capacity is exhausted or allocation fails, validation continues. Queries then use sequential replay.

Small selections can read indexed scalar dependencies. Broad selections replay sequentially. Both paths need earlier observations to establish state. A narrow vector slice still needs its whole source recipe. Overlapping recipes can share decoded source scalars. A reused waveform can therefore use its opening index without retaining full selected histories.

Historical producer output does not automatically define supported grammar. The issue-538 fixture omits required inner bounds. It fails instead of using the next type entry as a range. Empty derived layouts also need a checked expansion limit. Otherwise, a compact array of empty elements could create an unbounded hierarchy without a large value payload.
