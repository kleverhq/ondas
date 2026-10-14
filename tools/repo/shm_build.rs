//! Private, build-time SST2 export selection. No vendor library is loaded here.
use std::collections::HashSet;

pub(crate) fn exports(listing: &str) -> HashSet<&str> {
    listing
        .lines()
        .filter_map(|line| {
            let mut words = line.split_whitespace();
            let _address = words.next()?;
            matches!(words.next()?, "T" | "W")
                .then(|| words.next())
                .flatten()
        })
        .collect()
}

pub(crate) struct Selection {
    pub(crate) cpp_init: bool,
    pub(crate) alias_identity: bool,
}

pub(crate) fn select(exports: &HashSet<&str>) -> Result<Selection, &'static str> {
    for required in [
        "drOpenFile",
        "drClose",
        "da_get",
        "da_get_int",
        "da_get_real",
        "da_get_sstr",
        "da_get_sstr_pl",
        "da_get_xvalseq",
        "da_handle",
        "da_handle_pl",
        "da_iterate",
        "da_scan",
        "da_delete",
    ] {
        if !exports.contains(required) {
            return Err(required);
        }
    }
    let cpp_init = exports.contains("_Z8sst2Initv");
    if !cpp_init && !exports.contains("sst2Init") {
        return Err("sst2Init");
    }
    Ok(Selection {
        cpp_init,
        alias_identity: exports.contains("da_get_xref"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reader_exports() -> HashSet<&'static str> {
        exports(
            "00 T drOpenFile\n00 T drClose\n00 T da_get\n00 T da_get_int\n\
             00 T da_get_real\n00 T da_get_sstr\n00 T da_get_xref\n\
             00 T da_get_xvalseq\n00 T da_handle\n00 T da_handle_pl\n00 T da_iterate\n\
             00 T da_scan\n00 T da_delete\n00 T _Z8sst2Initv\n00 T da_get_sstr_pl",
        )
    }

    #[test]
    fn defined_exports_and_initializer_linkage() {
        let parsed = exports("00 T strong\n00 W weak\n U unresolved\n00 t hidden\n");
        assert_eq!(parsed, HashSet::from(["strong", "weak"]));
        let mut symbols = reader_exports();
        assert!(select(&symbols).unwrap().cpp_init);
        symbols.remove("_Z8sst2Initv");
        assert!(matches!(select(&symbols), Err("sst2Init")));
        symbols.insert("sst2Init");
        assert!(!select(&symbols).unwrap().cpp_init);
    }

    #[test]
    fn quoted_getter_is_required() {
        let mut symbols = reader_exports();
        assert!(select(&symbols).is_ok());
        symbols.remove("da_get_sstr_pl");
        assert!(matches!(select(&symbols), Err("da_get_sstr_pl")));
    }

    #[test]
    fn identity_is_optional() {
        let mut symbols = reader_exports();
        assert!(select(&symbols).unwrap().alias_identity);
        symbols.remove("da_get_xref");
        assert!(!select(&symbols).unwrap().alias_identity);
    }
}
