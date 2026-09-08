use super::{HierarchyPath, PathError};

fn error(offset: usize, message: impl Into<String>) -> PathError {
    PathError {
        offset,
        message: message.into(),
    }
}

pub(super) fn parse(text: &str) -> Result<Vec<String>, PathError> {
    let mut components = Vec::new();
    let mut offset = 0;
    while offset < text.len() {
        let start = offset;
        let component = match text.as_bytes()[offset] {
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
                    if character == '.' {
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
        components.push(component);
        if offset == text.len() {
            break;
        }
        if text.as_bytes()[offset] != b'.' {
            return Err(error(offset, "expected a component separator"));
        }
        offset += 1;
        if offset == text.len() {
            return Err(error(offset, "missing path component"));
        }
    }
    Ok(components)
}

pub(super) fn selector(text: &str) -> Result<Option<(HierarchyPath, u32, u32)>, PathError> {
    let Some(end) = text.strip_suffix(']') else {
        return Ok(None);
    };
    let Some(start) = end.rfind('[') else {
        return Ok(None);
    };
    let Some((msb, lsb)) = end[start + 1..].split_once(':') else {
        return Ok(None);
    };
    // Parsing the prefix ensures the suffix is outside a quoted or escaped name.
    let Ok(path) = HierarchyPath::parse(&text[..start]) else {
        return Ok(None);
    };
    let index = |value: &str, offset| {
        if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
            return Err(error(
                offset,
                "slice index must be an unsigned decimal integer",
            ));
        }
        value
            .parse::<u32>()
            .map_err(|_| error(offset, "slice index exceeds u32"))
    };
    Ok(Some((
        path,
        index(msb, start + 1)?,
        index(lsb, start + msb.len() + 2)?,
    )))
}
