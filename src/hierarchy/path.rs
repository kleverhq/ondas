use std::fmt::Write;

use super::{HierarchyPath, PathComponent, PathError, PathFormatError, SignalSelector};

fn error(offset: usize, message: impl Into<String>) -> PathError {
    PathError {
        offset,
        message: message.into(),
    }
}

fn array_index(text: &str) -> Option<i64> {
    let digits = text.strip_prefix('-').unwrap_or(text);
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

pub(super) fn index_suffix(text: &str) -> Option<i64> {
    array_index(text.strip_prefix('[')?.strip_suffix(']')?)
}

pub(super) fn split_index(text: &str) -> Option<(&str, i64)> {
    let (name, suffix) = text.strip_suffix(']')?.rsplit_once('[')?;
    if name.is_empty() || name.contains(['[', ']']) {
        return None;
    }
    Some((name, array_index(suffix)?))
}

pub(super) fn parse(text: &str, allow_projection: bool) -> Result<SignalSelector, PathError> {
    // Find the final range-shaped suffix. The component lexer still checks
    // whether that opening bracket belongs to a quoted or escaped name.
    let projection_suffix = text.strip_suffix(']').and_then(|prefix| {
        let (base, bounds) = prefix.rsplit_once('[')?;
        let (msb, lsb) = bounds.split_once(':')?;
        (!bounds.contains(']')).then_some((base.len(), msb, lsb))
    });
    let projection_start = projection_suffix.map(|(start, ..)| start);
    let mut components = Vec::new();
    let mut offset = 0;
    while offset < text.len() && projection_start != Some(offset) {
        let start = offset;
        let first = text.as_bytes()[offset];
        let name = match first {
            b'"' => {
                let input = &text[offset..];
                let mut strings = serde_json::Deserializer::from_str(input).into_iter::<String>();
                let component = strings
                    .next()
                    .expect("nonempty JSON input")
                    .map_err(|err| {
                        let position = if err.is_eof() {
                            input.len()
                        } else {
                            let line_start = input
                                .split_inclusive('\n')
                                .take(err.line().saturating_sub(1))
                                .map(str::len)
                                .sum::<usize>();
                            (line_start + err.column()).saturating_sub(1)
                        };
                        error(start + position, err.to_string())
                    })?;
                offset += strings.byte_offset();
                component
            }
            b'\\' => {
                offset += 1;
                let name_start = offset;
                for character in text[offset..].chars() {
                    if character.is_whitespace() {
                        break;
                    }
                    if !character.is_ascii() || character.is_control() {
                        return Err(error(offset, "escaped identifiers require printable ASCII"));
                    }
                    offset += character.len_utf8();
                }
                if offset == name_start {
                    return Err(error(offset, "empty escaped identifier"));
                }
                if offset == text.len() {
                    return Err(error(
                        offset,
                        "escaped identifier requires terminating whitespace",
                    ));
                }
                let component = text[name_start..offset].to_owned();
                while let Some(character) =
                    text[offset..].chars().next().filter(|c| c.is_whitespace())
                {
                    offset += character.len_utf8();
                }
                component
            }
            _ => {
                for character in text[offset..].chars() {
                    if character == '.' || projection_start == Some(offset) {
                        break;
                    }
                    if character.is_whitespace()
                        || character.is_control()
                        || matches!(character, '"' | '\\')
                    {
                        return Err(error(offset, "quote or escape this component"));
                    }
                    offset += character.len_utf8();
                }
                if offset == start {
                    return Err(error(offset, "empty path component"));
                }
                text[start..offset].to_owned()
            }
        };
        let component = if matches!(first, b'"' | b'\\') {
            let index = if projection_start != Some(offset)
                && let Some(suffix) = text[offset..].strip_prefix('[')
            {
                let end = suffix
                    .find(']')
                    .ok_or_else(|| error(offset, "unterminated array index"))?;
                let index = index_suffix(&text[offset..offset + end + 2])
                    .ok_or_else(|| error(offset + 1, "invalid array index"))?;
                offset += end + 2;
                Some(index)
            } else {
                None
            };
            PathComponent {
                name,
                index,
                name_was_escaped: first == b'\\',
            }
        } else {
            PathComponent::from_index_spelling(name)
        };
        components.push(component);
        if projection_start == Some(offset) || offset == text.len() {
            break;
        }
        if text.as_bytes()[offset] != b'.' {
            return Err(error(offset, "expected a component separator"));
        }
        offset += 1;
        if projection_start == Some(offset) || offset == text.len() {
            return Err(error(offset, "missing path component"));
        }
    }
    let projection = if let Some((start, msb, lsb)) = projection_suffix
        && start == offset
    {
        if !allow_projection {
            return Err(error(offset, "projection is not part of an exact path"));
        }
        Some((
            projection_index(msb, offset + 1)?,
            projection_index(lsb, offset + msb.len() + 2)?,
        ))
    } else {
        None
    };
    Ok(SignalSelector::new(
        HierarchyPath { components },
        projection,
    ))
}

pub(super) fn simple_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    bytes.next().is_some_and(|byte| byte.is_ascii_alphabetic())
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

pub(super) fn to_verilog(components: &[PathComponent]) -> Result<String, PathFormatError> {
    let mut output = String::new();
    for component in components {
        let name = component.name();
        if name.is_empty() || !name.bytes().all(|b| (33..=126).contains(&b)) {
            return Err(PathFormatError::NotRepresentable {
                component: name.to_owned(),
            });
        }
        if !output.is_empty() {
            output.push('.');
        }
        if simple_name(name) && !component.name_was_escaped {
            output.push_str(name);
        } else {
            output.push('\\');
            output.push_str(name);
            output.push(' ');
        }
        if let Some(index) = component.index() {
            write!(&mut output, "[{index}]").expect("writing to a String succeeds");
        }
    }
    Ok(output)
}

fn projection_index(value: &str, offset: usize) -> Result<u32, PathError> {
    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
        return Err(error(
            offset,
            "slice index must be an unsigned decimal integer",
        ));
    }
    value
        .parse()
        .map_err(|_| error(offset, "slice index exceeds u32"))
}
