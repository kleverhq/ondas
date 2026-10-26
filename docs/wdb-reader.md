# WDB storage windows and tick boundaries

WDB declarations and value addresses are separate structures. `src/backends/wdb/` reads simulation containers with DBG 004, 005 and 006. Crate Rustdoc defines supported values and public limits.

## A marker does not prove availability

The recorded-object marker is not a complete list of readable declarations. Some unmarked ports have recorded bytes. Keep their declarations and read the actual address ranges. A source value stays Missing until every required storage byte has an observation. Do not fill missing storage with zero or apply concat's explicit Missing-to-X rule.

A physical recipe contains an address, storage length, bit offset and codec. Share history identity only for equal recipes. Equal values do not prove aliases. Generated array and record views have no invented native reader spelling. A dot inside a native path component remains literal.

## Pages are not whole values

A compressed page expands to 10,240 bytes. Its records update ranges within a 2,048-byte arena. A value can span records, pages and arenas. Writes at one tick must complete across all selected arenas before decoding the final value. There is no order between different arenas at the same tick; each arena retains its own write order.

SystemVerilog bits use pairs of 32-bit words. The value word and state word together distinguish 0, 1, Z and X. A partial write can update one pair inside a wider value. VHDL logic values use source enum ordinals. Character enum labels do not make a sample a string. Named enum cells keep their full stored width.

## A point certificate is conditional

The normal walk keeps one inflated page per selected arena and current selected storage. It does not retain full histories. A reverse point read assumes native monotone page times to locate candidate pages. It checks decoded pages and crossed or probed boundaries. It does not validate unvisited adjacent pages or the complete source body.

The reverse path has a storage window of at most 2,048 bytes and at most two arena cursors per selected value. It collects each byte's final write within one complete tick. A different preceding complete value proves the last net-change time. Equal complete values extend the search backwards. Without a predecessor, first establishment has no known change time. An incomplete tick, an empty probed page, a larger value or a slice uses the chronological path. Compositions keep the common engine's chronological path.

A reused selection can retain bounded normalized snapshots. Advancing from a snapshot still reconstructs raw prefix bytes; it skips earlier value decoding, not prefix I/O. A repeated completed point can avoid source access. Break, errors and unwinding discard provisional query state.
