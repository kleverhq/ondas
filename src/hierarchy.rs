#[cfg(test)]
mod tests;

mod path;

use std::{
    collections::HashMap,
    fmt,
    str::FromStr,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
};

use crate::{LookupError, PathError, PathFormatError, Result, SliceError};

/// An owned, immutable sequence of exact hierarchy components.
///
/// Identity is the exact component sequence. Escaping affects only text
/// representations. Paths start at the hierarchy root. Relative paths and the
/// navigation components `.` and `..` are not supported.
///
/// # Accepted text forms
///
/// - Ordinary components: `tb.dut.data`.
/// - SystemVerilog escaped identifiers: `tb.\gen.blk[0] .data`. A leading
///   backslash starts the identifier. Whitespace ends it. Embedded dots and
///   brackets do not split the path.
/// - Ondas quoted components: `tb."name with space".data`,
///   `tb."name.with.dots".data`, or `tb."имя сигнала".data`. These represent names
///   outside SystemVerilog identifier syntax.
///
/// # Quoted components and canonical output
///
/// Each quoted component uses JSON string syntax. Accepted escapes are `\"`,
/// `\\`, `\/`, `\b`, `\f`, `\n`, `\r`, `\t`, and `\uXXXX` with exactly four
/// hexadecimal digits. Unicode surrogate pairs decode to one Unicode scalar.
/// Unpaired surrogates are invalid. Unknown escapes such as `\q`, incomplete
/// escapes, and unescaped `U+0000..=U+001F` characters produce [`PathError`]. These
/// rules apply inside quotes, not to SystemVerilog escaped identifiers.
///
/// [`Display`](fmt::Display) produces canonical Ondas syntax. Parsing that output
/// recovers the same components. Empty components and components containing
/// separators, whitespace, brackets, quotes, backslashes, `U+0000..=U+001F`, or
/// slice-like spelling are quoted. Inside quotes, canonical output uses `\"`
/// and `\\`, the short escapes `\b`, `\f`, `\n`, `\r`, and `\t`, and lowercase
/// `\u00xx` for the remaining `U+0000..=U+001F` characters. All other characters,
/// including `/` and non-ASCII Unicode, are written literally. Output does not
/// normalize Unicode. Thus `tb."a\u000Ab"` canonicalizes to `tb."a\nb"`.
///
/// [`Self::to_verilog`] instead produces lossless SystemVerilog-compatible text
/// or a [`PathFormatError`].
///
/// # Path versus selector
///
/// Brackets do not specify array indices or bit slices here. `tb.mem[0]` may name
/// an actual declaration. Only [`Hierarchy::signal`] interprets an optional
/// trailing `[msb:lsb]` as a projection after exact lookup fails.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct HierarchyPath {
    components: Vec<String>,
}

impl HierarchyPath {
    /// Parses the accepted text forms described on [`HierarchyPath`].
    ///
    /// Returns [`PathError`] with a byte offset for invalid syntax. This parses
    /// exact components, not a signal-slice expression.
    pub fn parse(text: &str) -> std::result::Result<Self, PathError> {
        path::parse(text).map(|components| Self { components })
    }

    /// Builds a path from exact, unescaped hierarchy components.
    ///
    /// Component contents are names, not text to parse or unescape.
    pub fn from_components<I, S>(components: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            components: components.into_iter().map(Into::into).collect(),
        }
    }

    /// Returns the exact hierarchy components in order.
    pub fn components(&self) -> impl ExactSizeIterator<Item = &str> + '_ {
        self.components.iter().map(String::as_str)
    }

    /// Returns the number of hierarchy components.
    pub fn len(&self) -> usize {
        self.components.len()
    }

    /// Returns whether the path has no components.
    pub fn is_empty(&self) -> bool {
        self.components.is_empty()
    }

    /// Returns the final component, if any.
    pub fn name(&self) -> Option<&str> {
        self.components.last().map(String::as_str)
    }

    /// Returns the path without its final component, if any.
    pub fn parent(&self) -> Option<Self> {
        self.components.split_last().map(|(_, parent)| Self {
            components: parent.to_vec(),
        })
    }

    /// Returns a new path with one exact component appended, leaving this path unchanged.
    ///
    /// The component is not parsed as a path or selector.
    pub fn join(&self, component: impl Into<String>) -> Self {
        let mut path = self.clone();
        path.components.push(component.into());
        path
    }

    /// Formats the path losslessly using SystemVerilog-compatible spelling.
    ///
    /// Returns [`PathFormatError::NotRepresentable`] if a component cannot be
    /// represented without loss. Ondas quoted syntax remains available through
    /// [`Display`](fmt::Display).
    ///
    /// Every component is escaped, including keywords and simple identifiers.
    pub fn to_verilog(&self) -> std::result::Result<String, PathFormatError> {
        let mut output = String::new();
        for component in &self.components {
            // IEEE 1800-2017 5.6.1: printable ASCII, terminated by whitespace.
            if component.is_empty() || !component.bytes().all(|b| (33..=126).contains(&b)) {
                return Err(PathFormatError::NotRepresentable {
                    component: component.clone(),
                });
            }
            if !output.is_empty() {
                output.push('.');
            }
            output.push('\\');
            output.push_str(component);
            output.push(' ');
        }
        Ok(output)
    }
}

impl FromStr for HierarchyPath {
    type Err = PathError;

    /// Parses a root-based hierarchy path.
    fn from_str(text: &str) -> std::result::Result<Self, Self::Err> {
        Self::parse(text)
    }
}

impl fmt::Display for HierarchyPath {
    /// Formats the path using canonical, round-trippable Ondas syntax.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, component) in self.components.iter().enumerate() {
            if index != 0 {
                formatter.write_str(".")?;
            }
            if component.is_empty()
                || component.chars().any(|c| {
                    c.is_whitespace() || c.is_control() || matches!(c, '.' | '[' | ']' | '"' | '\\')
                })
            {
                formatter.write_str(&serde_json::to_string(component).map_err(|_| fmt::Error)?)?;
            } else {
                formatter.write_str(component)?;
            }
        }
        Ok(())
    }
}

/// An immutable waveform hierarchy that can be cloned.
///
/// [`Scope`] and [`Variable`] are borrowed views. [`Signal`] is a copyable query
/// handle. Clone the hierarchy before retaining views across mutable queries on
/// its [`Waveform`](crate::Waveform). See the example on that type.
///
/// Declarations and histories are distinct. [`Self::variables`] includes aliases.
/// [`Self::signals`] lists whole histories with distinct handles. A reader can
/// leave aliases separate when it cannot establish their shared history.
/// Lookup failures use [`LookupError`], not opening or backend errors.
///
/// Scope and variable equality compares object identity, not paths or metadata.
/// A clone keeps the same objects. An independent open has different objects,
/// even when it reads the same file. Use equality to match a parent returned by
/// traversal with a retained scope. Equal paths can refer to distinct objects.
///
/// ```
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let source = b"$scope module top $end $var wire 1 ! flag $end \
///     $upscope $end $enddefinitions $end #0 1!";
/// let wave = ondas::open_bytes("tree.vcd", source.as_slice().into())?;
/// let hierarchy = wave.hierarchy();
/// let retained = hierarchy.clone();
/// let root = retained.scope("top")?;
/// let flag = hierarchy.variable("top.flag")?;
/// assert!(flag.parent().as_ref() == Some(&root));
/// assert!(flag == retained.variable("top.flag")?);
/// # Ok(())
/// # }
/// ```
#[derive(Clone)]
pub struct Hierarchy {
    data: Arc<HierarchyData>,
}

pub(crate) struct ScopeData {
    pub(crate) name: String,
    pub(crate) name_was_escaped: bool,
    pub(crate) parent: Option<usize>,
    pub(crate) kind: String,
    pub(crate) definition_name: Option<String>,
    pub(crate) packing: Option<Packing>,
    pub(crate) is_hidden: bool,
}

pub(crate) struct VariableData {
    pub(crate) name: String,
    pub(crate) reader_name: Option<String>,
    pub(crate) name_was_escaped: bool,
    pub(crate) parent: Option<usize>,
    pub(crate) kind: String,
    pub(crate) direction: Direction,
    pub(crate) range: Option<BitRange>,
    pub(crate) is_constant: bool,
    pub(crate) type_name: Option<String>,
    pub(crate) signedness: Option<Signedness>,
    pub(crate) logic_domain: Option<LogicDomain>,
    pub(crate) enumeration: Option<EnumerationData>,
    pub(crate) signal: Option<usize>,
}

pub(crate) struct EnumerationData {
    pub(crate) name: Option<String>,
    pub(crate) variants: Vec<(String, String)>,
}

impl EnumerationData {
    /// Expose only keys that can be matched to the whole stored bit value.
    pub(crate) fn for_encoding(
        name: Option<String>,
        mut variants: Vec<(String, String)>,
        encoding: Encoding,
    ) -> Option<Self> {
        let Encoding::Bits { width } = encoding else {
            return None;
        };
        if variants.is_empty()
            || !variants.iter().all(|(key, _)| {
                crate::BitsRef::from_ascii(key.as_bytes()).is_some_and(|bits| bits.width() == width)
            })
        {
            return None;
        }
        for (key, _) in &mut variants {
            key.make_ascii_lowercase();
        }
        Some(Self { name, variants })
    }
}

struct HierarchyData {
    source: u64,
    scopes: Vec<ScopeData>,
    variables: Vec<VariableData>,
    encodings: Vec<Encoding>,
    first_variable_lookup: OnceLock<()>,
    variables_by_path: OnceLock<HashMap<HierarchyPath, (usize, usize)>>,
}

static NEXT_SOURCE: AtomicU64 = AtomicU64::new(1);

/// A scope or variable encountered during hierarchy traversal.
pub enum Item<'h> {
    /// A hierarchy scope.
    Scope(Scope<'h>),
    /// A variable declaration.
    Variable(Variable<'h>),
}

/// A borrowed view of a scope in a hierarchy.
///
/// Equality identifies one scope in one source hierarchy. Views from a
/// [`Hierarchy::clone`] compare equal to the original views. Distinct scopes
/// compare unequal even when their paths and metadata are equal. Views from
/// independent opens compare unequal.
pub struct Scope<'h> {
    hierarchy: &'h Hierarchy,
    index: usize,
}

/// A borrowed view of a variable declaration in a hierarchy.
///
/// Equality identifies one declaration in one source hierarchy. Views from a
/// [`Hierarchy::clone`] compare equal to the original views. Alias declarations
/// compare unequal even when they share a [`Signal`]. Equal paths or metadata do
/// not make declarations equal. Views from independent opens compare unequal.
pub struct Variable<'h> {
    hierarchy: &'h Hierarchy,
    index: usize,
}

impl PartialEq<Scope<'_>> for Scope<'_> {
    fn eq(&self, other: &Scope<'_>) -> bool {
        self.hierarchy.data.source == other.hierarchy.data.source && self.index == other.index
    }
}

impl Eq for Scope<'_> {}

impl PartialEq<Variable<'_>> for Variable<'_> {
    fn eq(&self, other: &Variable<'_>) -> bool {
        self.hierarchy.data.source == other.hierarchy.data.source && self.index == other.index
    }
}

impl Eq for Variable<'_> {}

/// An opaque waveform signal handle with an optional bit projection.
///
/// A handle can have [`Encoding::Unsupported`]. Value queries then return
/// [`Error::UnsupportedSignal`](crate::Error::UnsupportedSignal).
///
/// Whole aliases compare equal when the reader establishes their shared history.
/// Otherwise, declarations have separate handles. Unequal handles do not prove
/// independent histories. Distinct histories have distinct whole handles.
/// Handles belong to their source hierarchy and waveform.
/// Using one with another source returns [`Error::InvalidSignal`](crate::Error::InvalidSignal).
/// Backend-local identifiers and projection representation are not public contracts.
///
/// A projection has its own observed history. Only changes to the selected bits
/// appear in traces and scans. For projected samples, `changed_at` identifies the
/// last change of the projected value, not unrelated activity in its base signal.
/// It is `None` if the backend cannot establish that time reliably.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Signal {
    source: u64,
    index: usize,
    base_encoding: Encoding,
    encoding: Encoding,
    lsb: u32,
}

impl Hierarchy {
    // Reader adaptation supplies normalized kinds, valid indices, and acyclic scope parents.
    pub(crate) fn new(
        scopes: Vec<ScopeData>,
        variables: Vec<VariableData>,
        encodings: Vec<Encoding>,
    ) -> Self {
        let source = NEXT_SOURCE
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .expect("hierarchy source IDs exhausted");
        Self {
            data: Arc::new(HierarchyData {
                source,
                scopes,
                variables,
                encodings,
                first_variable_lookup: OnceLock::new(),
                variables_by_path: OnceLock::new(),
            }),
        }
    }

    pub(crate) fn signal_at(&self, index: usize) -> Signal {
        let encoding = self.data.encodings[index];
        Signal {
            source: self.data.source,
            index,
            base_encoding: encoding,
            encoding,
            lsb: 0,
        }
    }

    pub(crate) fn validate(&self, signal: Signal) -> Result<usize> {
        let valid = signal.source == self.data.source
            && self.data.encodings.get(signal.index) == Some(&signal.base_encoding)
            && match (signal.base_encoding, signal.encoding) {
                (Encoding::Bits { width: base }, Encoding::Bits { width }) => {
                    (width > 0 || signal == signal.base())
                        && signal.lsb.checked_add(width).is_some_and(|end| end <= base)
                }
                (base, current) => base == current && signal.lsb == 0,
            };
        if valid {
            Ok(signal.index)
        } else {
            Err(crate::Error::InvalidSignal { signal })
        }
    }

    fn children_of(&self, parent: Option<usize>) -> impl Iterator<Item = Item<'_>> + '_ {
        self.scopes()
            .filter(move |scope| scope.data().parent == parent)
            .map(Item::Scope)
            .chain(
                self.variables()
                    .filter(move |variable| variable.data().parent == parent)
                    .map(Item::Variable),
            )
    }

    fn scope_path_at(&self, mut index: Option<usize>) -> HierarchyPath {
        let mut components = Vec::new();
        while let Some(current) = index {
            let scope = &self.data.scopes[current];
            components.push(scope.name.clone());
            index = scope.parent;
        }
        components.reverse();
        HierarchyPath { components }
    }

    /// Iterates over root scopes and variables.
    pub fn roots(&self) -> impl Iterator<Item = Item<'_>> + '_ {
        self.children_of(None)
    }

    /// Iterates over all scopes in the hierarchy.
    pub fn scopes(&self) -> impl Iterator<Item = Scope<'_>> + '_ {
        (0..self.data.scopes.len()).map(|index| Scope {
            hierarchy: self,
            index,
        })
    }

    /// Iterates over all declarations, including signal aliases.
    pub fn variables(&self) -> impl Iterator<Item = Variable<'_>> + '_ {
        (0..self.data.variables.len()).map(|index| Variable {
            hierarchy: self,
            index,
        })
    }

    /// Iterates over whole signals with distinct handles in the hierarchy.
    ///
    /// Aliases share a handle only when the reader establishes their shared history.
    pub fn signals(&self) -> impl Iterator<Item = Signal> + '_ {
        (0..self.data.encodings.len()).map(|index| self.signal_at(index))
    }

    /// Parses and resolves an exact scope path.
    pub fn scope(&self, text: &str) -> std::result::Result<Scope<'_>, LookupError> {
        self.scope_path(&HierarchyPath::parse(text)?)
    }

    /// Parses and resolves an exact variable path.
    pub fn variable(&self, text: &str) -> std::result::Result<Variable<'_>, LookupError> {
        self.variable_path(&HierarchyPath::parse(text)?)
    }

    /// Resolves an exact variable path first, then an optional trailing bit slice.
    ///
    /// The lookup first treats the entire selector as an exact variable path. If
    /// that variable is not found, one trailing `[msb:lsb]` can specify a
    /// [`Signal::slice`]. An actual variable named `data[7:0]` therefore takes
    /// priority over slicing `data`. `[n]` is never a slice. Select one bit with `[n:n]`.
    ///
    /// For example, `tb.mem[0]` names a whole signal, while `tb.mem[0][7:0]`
    /// can slice it. Use [`Self::signal_path`] followed by [`Signal::slice`] to
    /// avoid ambiguity. This is not an expression language or a dynamic index.
    ///
    /// Returns [`LookupError`] for invalid syntax, missing or ambiguous metadata,
    /// a declaration without a history, or an invalid slice.
    pub fn signal(&self, selector: &str) -> std::result::Result<Signal, LookupError> {
        let exact = HierarchyPath::parse(selector)
            .map_err(LookupError::from)
            .and_then(|path| self.signal_path(&path));
        if !matches!(
            &exact,
            Err(LookupError::NotFound { .. } | LookupError::InvalidPath(_))
        ) {
            return exact;
        }
        if let Some((path, msb, lsb)) = path::selector(selector)? {
            return Ok(self.signal_path(&path)?.slice(msb, lsb)?);
        }
        exact
    }

    /// Resolves an already parsed exact scope path.
    ///
    /// Returns [`LookupError::Ambiguous`] when distinct scopes share the path.
    /// Those scopes remain accessible through [`Self::scopes`] and [`Scope::children`].
    pub fn scope_path(&self, path: &HierarchyPath) -> std::result::Result<Scope<'_>, LookupError> {
        let mut matches = self
            .scopes()
            .filter(|scope| Some(scope.name()) == path.name() && scope.path() == *path);
        let first = matches.next();
        match (first, matches.count()) {
            (None, _) => Err(LookupError::NotFound { path: path.clone() }),
            (Some(scope), 0) => Ok(scope),
            (_, remaining) => Err(LookupError::Ambiguous {
                path: path.clone(),
                matches: remaining + 1,
            }),
        }
    }

    /// Resolves an already parsed exact variable path.
    pub fn variable_path(
        &self,
        path: &HierarchyPath,
    ) -> std::result::Result<Variable<'_>, LookupError> {
        if self.data.variables_by_path.get().is_none()
            && self.data.first_variable_lookup.set(()).is_ok()
        {
            let mut matches = self.variables().filter(|variable| {
                Some(variable.name()) == path.name() && variable.path() == *path
            });
            let first = matches.next();
            return match (first, matches.count()) {
                (None, _) => Err(LookupError::NotFound { path: path.clone() }),
                (Some(variable), 0) => Ok(variable),
                (_, remaining) => Err(LookupError::Ambiguous {
                    path: path.clone(),
                    matches: remaining + 1,
                }),
            };
        }
        let variables_by_path = self.data.variables_by_path.get_or_init(|| {
            let mut index = HashMap::with_capacity(self.data.variables.len());
            for variable in self.variables() {
                let entry = index.entry(variable.path()).or_insert((variable.index, 0));
                entry.1 += 1;
            }
            index
        });
        match variables_by_path.get(path) {
            None => Err(LookupError::NotFound { path: path.clone() }),
            Some(&(index, 1)) => Ok(Variable {
                hierarchy: self,
                index,
            }),
            Some(&(_, matches)) => Err(LookupError::Ambiguous {
                path: path.clone(),
                matches,
            }),
        }
    }

    /// Resolves an already parsed exact variable path to a whole signal.
    ///
    /// Never interprets brackets as a slice. Returns [`LookupError::NoSignal`]
    /// when the declaration exists but has no queryable history.
    pub fn signal_path(&self, path: &HierarchyPath) -> std::result::Result<Signal, LookupError> {
        self.variable_path(path)?
            .signal()
            .ok_or_else(|| LookupError::NoSignal { path: path.clone() })
    }

    /// Iterates over declarations that share a signal valid for this hierarchy.
    ///
    /// This includes the reader's confirmed aliases. Unknown alias relationships
    /// remain separate and do not appear in this iterator.
    ///
    /// For a sliced handle, returns the declarations of its base whole signal,
    /// ignoring the projection. Slicing does not create declarations. Whole,
    /// sliced, and repeatedly sliced handles with the same base return the same
    /// declarations. They retain their original metadata and declaration ranges.
    ///
    /// A handle from another hierarchy or waveform produces
    /// [`Error::InvalidSignal`](crate::Error::InvalidSignal).
    pub fn aliases(&self, signal: Signal) -> Result<impl Iterator<Item = Variable<'_>> + '_> {
        let index = self.validate(signal)?;
        Ok(self
            .variables()
            .filter(move |variable| variable.data().signal == Some(index)))
    }
}

impl<'h> Scope<'h> {
    fn data(&self) -> &'h ScopeData {
        &self.hierarchy.data.scopes[self.index]
    }

    /// Returns the scope's exact local name.
    pub fn name(&self) -> &'h str {
        &self.data().name
    }

    /// Returns the scope's owned, root-based path.
    pub fn path(&self) -> HierarchyPath {
        self.hierarchy.scope_path_at(Some(self.index))
    }

    /// Returns the containing scope, or `None` for a root scope.
    pub fn parent(&self) -> Option<Scope<'h>> {
        self.data().parent.map(|index| Scope {
            hierarchy: self.hierarchy,
            index,
        })
    }

    /// Iterates over the scope's direct child scopes and variables.
    pub fn children(&self) -> impl Iterator<Item = Item<'h>> + '_ {
        self.hierarchy.children_of(Some(self.index))
    }

    /// Returns the canonical lower-case scope kind.
    ///
    /// Common kinds are normalized. Unknown vendor kinds retain a namespaced
    /// spelling. They are not forced into a closed enum.
    pub fn kind(&self) -> &'h str {
        &self.data().kind
    }

    /// Returns whether the reader supplied a Verilog escape marker for this name.
    ///
    /// The marker is excluded from [`Self::name`] and path identity. Add one
    /// backslash before the name and terminate it with whitespace to form an
    /// escaped Verilog identifier.
    /// Terminating whitespace is not retained. Merged scopes retain the first
    /// declaration's spelling. This records available reader syntax, not whether
    /// the name needs escaping as a path. A reader that supplies logical names
    /// directly may not retain the source spelling and returns `false`.
    pub fn name_was_escaped(&self) -> bool {
        self.data().name_was_escaped
    }

    /// Returns whether the source explicitly marks this scope as hidden.
    ///
    /// Hidden scopes and their contents remain accessible. This flag is local
    /// to the scope and is not inherited. To hide a subtree, also check
    /// its ancestors. Returns `false` when the backend has no hidden-scope flag.
    pub fn is_hidden(&self) -> bool {
        self.data().is_hidden
    }

    /// Returns the defining module, entity, or equivalent name when available.
    pub fn definition_name(&self) -> Option<&'h str> {
        self.data().definition_name.as_deref()
    }

    /// Returns the scope's packing metadata when applicable.
    pub fn packing(&self) -> Option<Packing> {
        self.data().packing
    }
}

/// Separates a reader-provided Verilog escape marker from logical identity.
pub(crate) fn normalize_name(mut name: String) -> (String, bool) {
    let escaped = name.starts_with('\\');
    if escaped {
        name.remove(0);
    }
    (name, escaped)
}

impl<'h> Variable<'h> {
    fn data(&self) -> &'h VariableData {
        &self.hierarchy.data.variables[self.index]
    }

    /// Returns the declaration's exact logical local name.
    ///
    /// Recognized Verilog escape syntax is excluded. Literal backslashes,
    /// brackets, dots and whitespace remain component data. Use this name for
    /// lookup; do not parse [`Self::reader_name`] to construct a path.
    pub fn name(&self) -> &'h str {
        &self.data().name
    }

    /// Returns the reader-provided local declaration spelling, if retained.
    ///
    /// This diagnostic spelling can include escape syntax and printed ranges.
    /// FSDB retains callback spelling. WLF retains recognized escaped spelling.
    /// WLF spelling includes the terminating whitespace.
    /// Merged duplicate declarations retain the first spelling; aliases retain
    /// their own. Availability is optional. This text does not define identity
    /// or lookup. Use [`Self::name`] and [`Self::path`] for ordinary addressing.
    pub fn reader_name(&self) -> Option<&'h str> {
        self.data().reader_name.as_deref()
    }

    /// Returns whether the reader supplied a Verilog escape marker for this name.
    ///
    /// The recognized marker is excluded from [`Self::name`]. Prefix the name
    /// with one backslash and terminate it with whitespace to form an escaped
    /// Verilog identifier. The name omits a separately printed declared range;
    /// an attached suffix on an escaped identifier remains literal.
    /// This flag does not affect identity or lookup. Merged repeated
    /// declarations retain the first spelling. Aliases retain their own flags.
    /// A reader that supplies logical names directly may not retain the source
    /// spelling and returns `false`. The flag does not prove that an unescaped
    /// identifier appeared in the original HDL.
    pub fn name_was_escaped(&self) -> bool {
        self.data().name_was_escaped
    }

    /// Returns the declaration's owned, root-based path.
    pub fn path(&self) -> HierarchyPath {
        self.hierarchy
            .scope_path_at(self.data().parent)
            .join(self.name())
    }

    /// Returns the containing scope, if any.
    pub fn parent(&self) -> Option<Scope<'h>> {
        self.data().parent.map(|index| Scope {
            hierarchy: self.hierarchy,
            index,
        })
    }

    /// Returns the declaration's whole signal handle, if it has one.
    ///
    /// Aliases return equal handles when the reader establishes their shared history.
    /// Otherwise, the declarations have separate handles. A declaration can exist
    /// without a waveform signal. An unsupported encoding still has a
    /// handle, but value queries return [`Error::UnsupportedSignal`](crate::Error::UnsupportedSignal).
    pub fn signal(&self) -> Option<Signal> {
        self.data()
            .signal
            .map(|index| self.hierarchy.signal_at(index))
    }

    /// Returns the canonical lower-case declaration kind.
    ///
    /// Common kinds are normalized. Unknown vendor kinds retain a namespaced
    /// spelling. Kind is separate from constancy and value encoding.
    pub fn kind(&self) -> &'h str {
        &self.data().kind
    }

    /// Returns the declaration's direction metadata.
    pub fn direction(&self) -> Direction {
        self.data().direction
    }

    /// Returns the original HDL declaration range, if present.
    pub fn range(&self) -> Option<BitRange> {
        self.data().range
    }

    /// Returns whether the declaration denotes a constant value.
    ///
    /// This is independent of kind and encoding. A parameter or generic may
    /// carry bits, a real value, or a string.
    pub fn is_constant(&self) -> bool {
        self.data().is_constant
    }

    /// Returns the declared type name when available.
    pub fn type_name(&self) -> Option<&'h str> {
        self.data().type_name.as_deref()
    }

    /// Returns known signed or unsigned interpretation of this declaration.
    ///
    /// `None` means unavailable or not applicable, for example for a real, string
    /// or event. It does not mean unsigned. This metadata is independent of shared
    /// [`Signal`] identity and is never inferred from observed values or names.
    /// Reading it does not load value histories.
    pub fn signedness(&self) -> Option<Signedness> {
        self.data().signedness
    }

    /// Returns the declaration's known logic domain, not the states observed so far.
    ///
    /// `None` means unavailable or not applicable. An observed `0`/`1` value
    /// does not establish a two-state domain. Aliases may have different metadata
    /// while sharing a [`Signal`]. Reading it does not load value histories.
    pub fn logic_domain(&self) -> Option<LogicDomain> {
        self.data().logic_domain
    }

    /// Returns enumeration metadata whose keys match the whole signal's bits.
    ///
    /// `None` means the reader cannot supply this mapping. It does not prove
    /// that the source declaration is not an enum. Keys describe the whole
    /// signal, not a slice. The reader does not pad, reverse or reinterpret keys
    /// to fit a declaration.
    pub fn enumeration(&self) -> Option<Enumeration<'h>> {
        self.data()
            .enumeration
            .as_ref()
            .map(|data| Enumeration { data })
    }
}

/// Known signedness of a variable declaration, independent of stored bit values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Signedness {
    /// A signed interpretation.
    Signed,
    /// An unsigned interpretation.
    Unsigned,
}

/// Known logic domain of a variable declaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum LogicDomain {
    /// Logic zero and one.
    TwoState,
    /// Logic zero, one, unknown and high impedance.
    FourState,
    /// The full nine-state domain represented by [`Logic`](crate::Logic).
    NineState,
}

/// Direction metadata for a variable declaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Direction {
    /// The direction is unavailable or unrecognized.
    Unknown,
    /// The declaration has an implicit direction.
    Implicit,
    /// An input declaration.
    Input,
    /// An output declaration.
    Output,
    /// A bidirectional declaration.
    InOut,
    /// A VHDL buffer declaration.
    Buffer,
    /// A VHDL linkage declaration.
    Linkage,
}

/// Packing metadata for a hierarchy scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Packing {
    /// A packed representation.
    Packed,
    /// An unpacked representation.
    Unpacked,
    /// A sparse representation.
    Sparse,
    /// A tagged packed representation.
    TaggedPacked,
}

/// The original index range of an HDL declaration.
///
/// The range may ascend or descend and may use arbitrary HDL indices. It preserves
/// declaration indices, independently of the normalized positions used by
/// [`Signal::slice`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BitRange {
    msb: i64,
    lsb: i64,
}

impl BitRange {
    /// Creates a declaration range preserving the supplied index order.
    pub const fn new(msb: i64, lsb: i64) -> Self {
        Self { msb, lsb }
    }

    /// Returns the declaration's `msb` index.
    pub const fn msb(self) -> i64 {
        self.msb
    }

    /// Returns the declaration's `lsb` index.
    pub const fn lsb(self) -> i64 {
        self.lsb
    }

    /// Returns the inclusive width of the range.
    ///
    /// # Panics
    ///
    /// Panics when the full `i64::MIN..=i64::MAX` span cannot fit in `u64`.
    pub const fn width(self) -> u64 {
        match self.msb.abs_diff(self.lsb).checked_add(1) {
            Some(width) => width,
            None => panic!("range width exceeds u64"),
        }
    }
}

/// Borrowed enumeration metadata for a variable declaration.
///
/// Each key is a [`BitsRef`](crate::BitsRef) with the whole signal's bit width.
/// Bit zero is the least-significant bit, as in a queried bit value. Match the
/// complete bit value with the key. Signedness and the HDL declaration range do
/// not change this comparison. All nine logic states can occur in keys; their
/// presence does not establish the declaration's domain.
/// A slice does not carry a projected enumeration.
///
/// Labels retain reader-provided spelling. They are not string sample values
/// and are not restricted to ASCII. Order, duplicate keys and duplicate labels
/// are preserved. A value need not have a label; a key can have multiple labels.
/// This metadata does not change the signal's encoding.
///
/// Compare keys with queried bits through the public logic-state iterators:
///
/// ```no_run
/// use ondas::{Sample, Time, Value};
/// # fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let mut wave = ondas::open("dump.fst")?;
/// let hierarchy = wave.hierarchy().clone();
/// let variable = hierarchy.variable("top.state")?;
/// if let (Some(signal), Some(table)) = (variable.signal(), variable.enumeration()) {
///     if let Sample::Value { value: Value::Bits(bits), .. } = wave.sample(signal, Time::ZERO)? {
///         for variant in table.variants() {
///             if variant.encoded.iter_msb().eq(bits.as_ref().iter_msb()) {
///                 println!("{}", variant.label);
///             }
///         }
///     }
/// }
/// # Ok(())
/// # }
/// ```
pub struct Enumeration<'h> {
    data: &'h EnumerationData,
}

impl<'h> Enumeration<'h> {
    /// Returns the enumeration type name when available.
    pub fn name(&self) -> Option<&'h str> {
        self.data.name.as_deref()
    }

    /// Iterates over borrowed whole-value bits and their reader-provided labels.
    pub fn variants(&self) -> impl Iterator<Item = EnumerationVariant<'h>> + '_ {
        self.data
            .variants
            .iter()
            .map(|(encoded, label)| EnumerationVariant {
                encoded: crate::BitsRef::from_validated_ascii(encoded.as_bytes()),
                label,
            })
    }
}

/// An encoded enumeration value and its source label.
pub struct EnumerationVariant<'h> {
    /// The whole signal's bit value, borrowed from hierarchy storage.
    ///
    /// The view remains valid for the hierarchy borrow. Use
    /// [`BitsRef::to_owned`](crate::BitsRef::to_owned) to retain an independent copy,
    /// or [`Display`](std::fmt::Display) to format it as an MSB-first logic string.
    pub encoded: crate::BitsRef<'h>,
    /// The corresponding reader-provided enumeration label spelling.
    pub label: &'h str,
}

/// The value encoding of a signal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Encoding {
    /// A bit-vector encoding.
    Bits {
        /// The bit-vector width.
        width: u32,
    },
    /// A real-number encoding.
    Real,
    /// A string encoding.
    String,
    /// An event occurrence encoding.
    Event,
    /// An encoding visible in the hierarchy but unsupported for queries.
    Unsupported,
}

impl Signal {
    pub(crate) fn index(self) -> usize {
        self.index
    }

    pub(crate) fn lsb(self) -> u32 {
        self.lsb
    }

    /// Returns the signal's value encoding.
    pub fn encoding(self) -> Encoding {
        self.encoding
    }

    /// Returns the current bit width for bit-vector signals, or `None` otherwise.
    ///
    /// For a slice, this is the projected width, not the base signal's width.
    pub fn width(self) -> Option<u32> {
        match self.encoding {
            Encoding::Bits { width } => Some(width),
            _ => None,
        }
    }

    /// Selects an inclusive normalized range from the current bit-vector value.
    ///
    /// Index zero is the least-significant, rightmost bit of the current value.
    /// These positions need not match [`Variable::range`]. The bounds must satisfy
    /// `msb >= lsb` and `msb < current_width`. Non-bit encodings return [`SliceError::NotBits`],
    /// reversed bounds return [`SliceError::InvalidRange`], and bounds outside
    /// the current width return [`SliceError::OutOfBounds`].
    ///
    /// Repeated slices compose relative to the current projection:
    /// `data.slice(31, 16)?.slice(7, 0)?` selects original bits `[23:16]`.
    /// [`Self::base`] removes the projection. Selecting the full current width
    /// returns the same handle. A full-width slice of a whole signal is whole.
    ///
    /// The resulting history filters out changes that leave the projected value
    /// unchanged. For base values `00000000` at tick 10, `00000001` at tick 20,
    /// and `10100001` at tick 30, slice `[7:4]` changes at 10 and 30, not 20.
    /// Its sample `changed_at` is the last projected change time, or `None` when
    /// that time cannot be determined reliably. It is not another bit's change time.
    /// Multiple slices of one base remain separate public selection entries.
    pub fn slice(self, msb: u32, lsb: u32) -> std::result::Result<Self, SliceError> {
        let width = self.width().ok_or(SliceError::NotBits)?;
        if msb < lsb {
            return Err(SliceError::InvalidRange { msb, lsb });
        }
        if msb >= width {
            return Err(SliceError::OutOfBounds { width, msb, lsb });
        }
        Ok(Self {
            encoding: Encoding::Bits {
                width: msb - lsb + 1,
            },
            lsb: self.lsb + lsb,
            ..self
        })
    }

    /// Returns the complete underlying signal without a bit projection.
    pub fn base(self) -> Self {
        Self {
            encoding: self.base_encoding,
            lsb: 0,
            ..self
        }
    }

    /// Returns whether this handle represents a bit projection.
    pub fn is_slice(self) -> bool {
        self != self.base()
    }
}
