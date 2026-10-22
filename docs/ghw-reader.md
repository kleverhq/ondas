# GHW source grammar and scalar recipes

GHW has no separate normative format specification. The implementation follows the GHDL producer/reader at [revision 2c451de76b07b73fd338c91d68fd5cefcb152512](https://github.com/ghdl/ghdl/tree/2c451de76b07b73fd338c91d68fd5cefcb152512), with independent fixture evidence. Grammar and supported layouts are in `src/backends/ghw/`; the public limits are in crate Rustdoc.

## Source conventions are not metadata

GHW stores no time resolution. Preserve ticks without adopting the native oracle's conventional 1 fs scale. The string table uses byte-preserving Latin-1 mapping for names and labels; this does not extend the ASCII sample-string contract.

Constrained enum subsets retain base ordinals rather than renumbering their labels. Character leaves remain enums, not strings. Physical values remain stored integers, not values scaled by a guessed unit. Only qualified one-dimensional bit/std_ulogic vectors become whole signals; other arrays retain readable element structure. Null vectors and null arrays preserve different declaration shapes without invented values.

## Whole values depend on ordered scalar recipes

A source value is assembled from an ordered scalar-ID recipe. Exact aliases can share that recipe, while reversed and overlapping recipes remain distinct. A whole value is Missing until all its source positions have observations. Do not apply the explicit concat Missing-to-X rule to an incomplete source recipe.

Equal ticks can cross snapshot and cycle sections. Indexed reads must combine all final scalar payloads for that tick before assembling a whole value. Delivering each change list independently would expose an intermediate value and can advance a projected change time incorrectly.

## Opening and seeking have different costs

Opening validates the whole body. Metadata-only reading uses that same path, unlike a header-only SDK operation. An optional bounded index stores scalar payload offsets and ticks, not values. Footer offsets do not drive seeking. If index capacity is exhausted or allocation fails, validation continues and queries use sequential replay.

Small selections can read indexed scalar dependencies; broad selections replay sequentially. Both still need earlier observations to establish state, and a narrow vector slice still requires its whole source recipe. Shared source scalars can be decoded once across overlapping recipes. A reused waveform can therefore benefit from its opening index without retaining full selected histories.

Historical producer output is not automatically a supported grammar. The issue-538 fixture omits required inner bounds and fails rather than treating the next type entry as a range. Empty derived layouts also need a checked expansion limit: a compact empty-element array can otherwise create an unbounded hierarchy without a large value payload.
