use std::collections::HashMap;

use crate::{Encoding, hierarchy::VariableData};

/// Returns declarations and unique MSB-first recipes over reader encoding indices.
/// A normalized declaration uses slot `encodings.len() + recipe_index`.
pub(super) fn normalize(
    mut variables: Vec<VariableData>,
    encodings: &[Encoding],
) -> (Vec<VariableData>, Vec<Box<[usize]>>) {
    let groups = groups(&variables, encodings);
    let mut definitions = Vec::new();
    let mut indices = HashMap::new();
    let mut retained = vec![true; variables.len()];
    for members in groups {
        let first = *members.iter().min().unwrap();
        let parts: Box<[usize]> = members
            .iter()
            .map(|&member| variables[member].signal.unwrap())
            .collect();
        let definition = if let Some(&index) = indices.get(&parts) {
            index
        } else {
            let index = definitions.len();
            indices.insert(parts.clone(), index);
            definitions.push(parts);
            index
        };
        for member in members {
            retained[member] = member == first;
        }
        variables[first].signal = Some(encodings.len() + definition);
        // A recorded span does not establish the original whole HDL declaration range.
        variables[first].range = None;
    }
    let variables = variables
        .into_iter()
        .zip(retained)
        .filter_map(|(variable, keep)| keep.then_some(variable))
        .collect();
    (variables, definitions)
}

// Group only parser-established ranges. Names are exact components, never syntax here.
fn groups(variables: &[VariableData], encodings: &[Encoding]) -> Vec<Vec<usize>> {
    let mut candidates = HashMap::<_, Vec<usize>>::new();
    for (index, variable) in variables.iter().enumerate() {
        candidates
            .entry((variable.parent, &variable.component))
            .or_default()
            .push(index);
    }
    let mut groups: Vec<_> = candidates
        .into_values()
        .filter_map(|mut members| {
            if members.len() < 2 {
                return None;
            }
            let first = &variables[members[0]];
            let mut ascending = None;
            let mut total = 0u32;
            for &index in &members {
                let variable = &variables[index];
                // VCD has implicit direction and no type metadata. Kind determines constancy.
                if variable.kind != first.kind
                    || variable.name_was_escaped != first.name_was_escaped
                {
                    return None;
                }
                let Encoding::Bits { width } = *encodings.get(variable.signal?)? else {
                    return None;
                };
                let range = variable.range?;
                if width == 0
                    || range.msb().abs_diff(range.lsb()).checked_add(1) != Some(u64::from(width))
                {
                    return None;
                }
                total = total.checked_add(width)?;
                if width > 1 {
                    let direction = range.msb() < range.lsb();
                    if ascending.is_some_and(|prior| prior != direction) {
                        return None;
                    }
                    ascending = Some(direction);
                }
            }
            // Scalar-only groups use higher-index-first. Mixed groups follow their chunks.
            let ascending = ascending.unwrap_or(false);
            members.sort_unstable_by_key(|&index| {
                let range = variables[index].range.unwrap();
                range.msb().min(range.lsb())
            });
            for pair in members.windows(2) {
                let left = variables[pair[0]].range.unwrap();
                let right = variables[pair[1]].range.unwrap();
                if left.msb().max(left.lsb()).checked_add(1) != Some(right.msb().min(right.lsb())) {
                    return None;
                }
            }
            if !ascending {
                members.reverse();
            }
            Some(members)
        })
        .collect();
    groups.sort_unstable_by_key(|members| *members.iter().min().unwrap());
    groups
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BitRange, Direction};

    fn variable(signal: usize, range: Option<(i64, i64)>) -> VariableData {
        VariableData {
            component: "bus".into(),
            reader_name: None,
            name_was_escaped: false,
            parent: None,
            kind: "wire".into(),
            direction: Direction::Implicit,
            range: range.map(|(msb, lsb)| BitRange::new(msb, lsb)),
            is_constant: false,
            type_name: None,
            signedness: None,
            logic_domain: None,
            enumeration: None,
            signal: Some(signal),
        }
    }

    #[test]
    fn only_complete_consistent_groups_are_confirmed() {
        let bits = [Encoding::Bits { width: 2 }; 2];
        for ranges in [[(3, 2), (0, -1)], [(3, 2), (2, 1)], [(3, 2), (0, 1)]] {
            assert!(
                groups(
                    &[variable(0, Some(ranges[0])), variable(1, Some(ranges[1]))],
                    &bits
                )
                .is_empty()
            );
        }
        let mut vars = vec![variable(0, Some((3, 2))), variable(1, Some((1, 0)))];
        assert_eq!(groups(&vars, &bits), [vec![0, 1]]);
        vars[1].kind = "reg".into();
        assert!(groups(&vars, &bits).is_empty());
        vars[1].kind = "wire".into();
        vars[1].parent = Some(0);
        assert!(groups(&vars, &bits).is_empty());
        vars[1].parent = None;
        vars[1].range = None;
        assert!(groups(&vars, &bits).is_empty());
        vars[1].range = Some(BitRange::new(1, 0));
        vars.push(variable(0, None));
        assert!(groups(&vars, &bits).is_empty());
    }

    #[test]
    fn widths_and_extreme_indices_use_checked_arithmetic() {
        let encodings = [
            Encoding::Bits { width: u32::MAX },
            Encoding::Bits { width: 1 },
        ];
        let end = i64::from(u32::MAX);
        assert!(
            groups(
                &[
                    variable(0, Some((end - 1, 0))),
                    variable(1, Some((end, end)))
                ],
                &encodings
            )
            .is_empty()
        );
        let bits = [Encoding::Bits { width: 2 }; 2];
        let vars = [
            variable(0, Some((i64::MIN, i64::MIN + 1))),
            variable(1, Some((i64::MIN + 2, i64::MIN + 3))),
        ];
        assert_eq!(groups(&vars, &bits), [vec![0, 1]]);
        assert!(groups(&vars, &[Encoding::Real, bits[1]]).is_empty());
        assert!(groups(&vars, &[Encoding::Bits { width: 1 }, bits[1]]).is_empty());
    }
}
