//! Private reuse of one fully validated GHW source.
//!
//! Opening a GHW file validates every record of its body. When an earlier open
//! already did that for byte-identical content, the immutable result can be
//! parked and taken again: the declarations, types, recipes, active scalar
//! table, body position and completed position index are reusable metadata.
//! Values, source bytes, input owners and query state are never parked.
use super::{hierarchy::Recipe, identity::ContentIdentity, index::Index, types::Types};
use crate::{Hierarchy, TimeSpan};

/// Upper bound for the retained graph of the one parked source, including nested
/// vector capacities of types, recipes, scalar IDs, the index and the declaration
/// lookup tables that a hierarchy builds on first use.
pub(super) const CACHE_BYTES: usize = 1024 * 1024 * 1024;

/// One complete result of a checked parse.
pub(super) struct Validated {
    /// Identity of the exact bytes the checked parse consumed. `None` when the
    /// pass could not bind a proof, in which case the result is never parked.
    pub(super) identity: Option<ContentIdentity>,
    /// Declaration data shared with every reuse; each logical open still takes a
    /// fresh [`Hierarchy::reopened`] identity from it.
    pub(super) hierarchy: Hierarchy,
    pub(super) types: Types,
    pub(super) recipes: Vec<Recipe>,
    pub(super) active: Vec<(u32, usize)>,
    pub(super) body_start: u64,
    pub(super) version: u8,
    pub(super) big_endian: bool,
    pub(super) time_span: Option<TimeSpan>,
    pub(super) index: Option<Index>,
    /// Bytes this result retains, so admission can refuse oversized sources.
    pub(super) retained_bytes: usize,
}

/// Retained bytes of one validated result.
///
/// Every vector payload is counted by the capacity it actually reserved, including
/// the nested source-ID lists of each recipe, because that is what the parked graph
/// keeps alive. Sizes saturate, so an absurd result is refused by the budget instead
/// of wrapping.
pub(super) fn retained_bytes(
    types: &Types,
    recipes: &[Recipe],
    recipe_slots: usize,
    active_slots: usize,
    hierarchy: &Hierarchy,
    index: Option<&Index>,
) -> usize {
    let mut bytes = std::mem::size_of::<Validated>();
    bytes = bytes.saturating_add(hierarchy.validated_bytes());
    bytes = bytes.saturating_add(types.retained_bytes());
    bytes = bytes.saturating_add(vec_bytes::<Recipe>(recipe_slots));
    for recipe in recipes {
        bytes = bytes.saturating_add(vec_bytes::<u32>(recipe.ids.capacity()));
    }
    bytes = bytes.saturating_add(vec_bytes::<(u32, usize)>(active_slots));
    if let Some(index) = index {
        bytes = bytes.saturating_add(index.retained_bytes());
    }
    bytes
}

fn vec_bytes<T>(capacity: usize) -> usize {
    capacity.saturating_mul(std::mem::size_of::<T>())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Encoding;
    use std::io::Cursor;

    fn empty_types() -> Types {
        let mut wire = crate::backends::ghw::wire::Wire::new(Box::new(Cursor::new(vec![
            b'T', b'Y', b'P', 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ])))
        .unwrap();
        Types::read(&mut wire, vec![String::new()]).unwrap()
    }

    #[test]
    fn accounting_counts_reserved_capacity_not_length() {
        let types = empty_types();
        let hierarchy = Hierarchy::new(Vec::new(), Vec::new(), Vec::new());
        let mut recipes = Vec::with_capacity(64);
        recipes.push(Recipe {
            ids: Vec::with_capacity(32),
            encoding: Encoding::Bits { width: 1 },
            scalar_type: 0,
            vector: false,
        });
        let mut active = Vec::with_capacity(128);
        active.push((1, 0));
        let counted = retained_bytes(
            &types,
            &recipes,
            recipes.capacity(),
            active.capacity(),
            &hierarchy,
            None,
        );
        let reserved = 64 * std::mem::size_of::<Recipe>()
            + 32 * std::mem::size_of::<u32>()
            + 128 * std::mem::size_of::<(u32, usize)>();
        assert!(counted >= reserved, "{counted} < {reserved}");
        // A length-based estimate would be two orders of magnitude smaller.
        assert!(
            counted
                > 10 * (std::mem::size_of::<Recipe>()
                    + std::mem::size_of::<u32>()
                    + std::mem::size_of::<(u32, usize)>())
        );
    }
}
