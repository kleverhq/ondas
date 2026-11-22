use std::fmt::Write;

use super::{HierarchyPath, PathComponent, PathError, PathFormatError};

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

pub(super) fn parse(text: &str) -> Result<Vec<PathComponent>, PathError> {
    let mut components = Vec::new();
    let mut offset = 0;
    while offset < text.len() {
        let start = offset;
        let explicit_name = matches!(text.as_bytes()[offset], b'"' | b'\\');
        let name = match text.as_bytes()[offset] {
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
        let component = if explicit_name {
            if let Some(suffix) = text[offset..].strip_prefix('[') {
                let end = suffix
                    .find(']')
                    .ok_or_else(|| error(offset, "unterminated array index"))?;
                let index = index_suffix(&text[offset..offset + end + 2])
                    .ok_or_else(|| error(offset + 1, "invalid array index"))?;
                offset += end + 2;
                PathComponent::indexed(name, index)
            } else {
                PathComponent::new(name)
            }
        } else {
            PathComponent::from_index_spelling(name)
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

fn simple_sv_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    bytes
        .next()
        .is_some_and(|byte| byte.is_ascii_alphabetic() || byte == b'_')
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'$'))
        && SV_KEYWORDS.binary_search(&name).is_err()
}

// IEEE 1800-2017, Table B.1.
#[rustfmt::skip]
const SV_KEYWORDS: &[&str] = &[
    "accept_on", "alias", "always", "always_comb", "always_ff", "always_latch", "and",
    "assert", "assign", "assume", "automatic", "before", "begin", "bind", "bins", "binsof",
    "bit", "break", "buf", "bufif0", "bufif1", "byte", "case", "casex", "casez", "cell",
    "chandle", "checker", "class", "clocking", "cmos", "config", "const", "constraint",
    "context", "continue", "cover", "covergroup", "coverpoint", "cross", "deassign", "default",
    "defparam", "design", "disable", "dist", "do", "edge", "else", "end", "endcase",
    "endchecker", "endclass", "endclocking", "endconfig", "endfunction", "endgenerate",
    "endgroup", "endinterface", "endmodule", "endpackage", "endprimitive", "endprogram",
    "endproperty", "endsequence", "endspecify", "endtable", "endtask", "enum", "event",
    "eventually", "expect", "export", "extends", "extern", "final", "first_match", "for",
    "force", "foreach", "forever", "fork", "forkjoin", "function", "generate", "genvar",
    "global", "highz0", "highz1", "if", "iff", "ifnone", "ignore_bins", "illegal_bins",
    "implements", "implies", "import", "incdir", "include", "initial", "inout", "input",
    "inside", "instance", "int", "integer", "interconnect", "interface", "intersect", "join",
    "join_any", "join_none", "large", "let", "liblist", "library", "local", "localparam",
    "logic", "longint", "macromodule", "matches", "medium", "modport", "module", "nand",
    "negedge", "nettype", "new", "nexttime", "nmos", "nor", "noshowcancelled", "not", "notif0",
    "notif1", "null", "or", "output", "package", "packed", "parameter", "pmos", "posedge",
    "primitive", "priority", "program", "property", "protected", "pull0", "pull1", "pulldown",
    "pullup", "pulsestyle_ondetect", "pulsestyle_onevent", "pure", "rand", "randc", "randcase",
    "randsequence", "rcmos", "real", "realtime", "ref", "reg", "reject_on", "release",
    "repeat", "restrict", "return", "rnmos", "rpmos", "rtran", "rtranif0", "rtranif1",
    "s_always", "s_eventually", "s_nexttime", "s_until", "s_until_with", "scalared",
    "sequence", "shortint", "shortreal", "showcancelled", "signed", "small", "soft", "solve",
    "specify", "specparam", "static", "string", "strong", "strong0", "strong1", "struct",
    "super", "supply0", "supply1", "sync_accept_on", "sync_reject_on", "table", "tagged",
    "task", "this", "throughout", "time", "timeprecision", "timeunit", "tran", "tranif0",
    "tranif1", "tri", "tri0", "tri1", "triand", "trior", "trireg", "type", "typedef", "union",
    "unique", "unique0", "unsigned", "until", "until_with", "untyped", "use", "uwire", "var",
    "vectored", "virtual", "void", "wait", "wait_order", "wand", "weak", "weak0", "weak1",
    "while", "wildcard", "wire", "with", "within", "wor", "xnor", "xor",
];

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
        if simple_sv_name(name) {
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
