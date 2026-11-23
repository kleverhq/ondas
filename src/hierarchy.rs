#[cfg(test)]
mod tests;

mod path;

use std::{
    cmp, fmt,
    fmt::Write,
    hash::{Hash, Hasher},
    str::FromStr,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
};

use crate::{LookupError, PathError, PathFormatError, Result, SliceError};

/// One logical hierarchy name and an optional array index.
///
/// Equality, ordering and hashing use the name and index. A literal
/// `u_reg[0]` and index `0` of `u_reg` are distinct components.
#[derive(Debug, Clone, Eq)]
pub struct PathComponent {
    name: String,
    index: Option<i64>,
    name_was_escaped: bool,
}

impl PathComponent {
    /// Creates a literal component. The name is used exactly as given.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            index: None,
            name_was_escaped: false,
        }
    }

    /// Creates a component for one element of an array.
    pub fn indexed(name: impl Into<String>, index: i64) -> Self {
        Self {
            name: name.into(),
            index: Some(index),
            name_was_escaped: false,
        }
    }

    /// Returns the logical name, without the array index.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the array index, if present.
    pub fn index(&self) -> Option<i64> {
        self.index
    }

    fn identity(&self) -> (&str, Option<i64>) {
        (&self.name, self.index)
    }

    fn with_escape(mut self, name_was_escaped: bool) -> Self {
        self.name_was_escaped = name_was_escaped;
        self
    }

    pub(crate) fn from_index_spelling(mut name: String) -> Self {
        if let Some((base, index)) = path::split_index(&name) {
            name.truncate(base.len());
            Self::indexed(name, index)
        } else {
            Self::new(name)
        }
    }

    pub(crate) fn from_reader_name(name: String, recognize_index: bool) -> Self {
        if recognize_index {
            Self::from_index_spelling(name)
        } else {
            Self::new(name)
        }
    }

    #[cfg(any(feature = "wdb-lib", feature = "wlf-lib"))]
    pub(crate) fn from_array_spelling(mut name: String, base_len: usize) -> Self {
        if let Some(index) = path::index_suffix(&name[base_len..]) {
            name.truncate(base_len);
            Self::indexed(name, index)
        } else {
            Self::new(name)
        }
    }
}

impl PartialEq for PathComponent {
    fn eq(&self, other: &Self) -> bool {
        self.identity() == other.identity()
    }
}

impl Ord for PathComponent {
    fn cmp(&self, other: &Self) -> cmp::Ordering {
        self.identity().cmp(&other.identity())
    }
}

impl PartialOrd for PathComponent {
    fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Hash for PathComponent {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.identity().hash(state);
    }
}

impl<T: Into<String>> From<T> for PathComponent {
    fn from(name: T) -> Self {
        Self::new(name)
    }
}

impl From<&PathComponent> for PathComponent {
    fn from(component: &PathComponent) -> Self {
        component.clone()
    }
}

/// The text style used to format a hierarchy path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathStyle {
    /// Canonical Ondas syntax. Quotes preserve any name.
    Logical,
    /// SystemVerilog escape notation. See [`HierarchyPath::format`].
    Sv,
}

/// An owned, immutable sequence of exact hierarchy components.
///
/// Identity includes each component's name and optional index. Exact lookup
/// starts at the root. [`Hierarchy::resolve_selector`] can qualify a selector
/// path relative to a scope. Navigation components `.` and `..` are unsupported.
///
/// # Accepted text forms
///
/// - Ordinary components: `tb.dut.data`.
/// - Indexed components: `tb.u_reg[0].data`. Quote or escape the name to
///   include literal brackets, as in `tb."u_reg[0]".data`.
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
/// [`Display`](fmt::Display) and [`PathStyle::Logical`] produce the same
/// canonical Ondas syntax. Parsing that output recovers the same components.
/// A simple name starts with an ASCII letter or an underscore. The remaining
/// characters are ASCII letters, digits, underscores or dollar signs.
/// Logical output quotes all other names.
/// The array index follows the name. Inside quotes, canonical output uses `\"`
/// and `\\`, the short escapes `\b`, `\f`, `\n`, `\r`, and `\t`, and lowercase
/// `\u00xx` for the remaining `U+0000..=U+001F` characters. All other characters,
/// including `/` and non-ASCII Unicode, are written literally. Output does not
/// normalize Unicode. Thus `tb."a\u000Ab"` canonicalizes to `tb."a\nb"`.
///
/// Use [`Self::format`] to select a style at runtime.
///
/// # Path versus selector
///
/// A signed decimal index selects an array element at one hierarchy level.
/// Use [`SignalSelector`] for a trailing `[msb:lsb]` bit projection. An exact
/// path rejects this suffix. Quote or escape the name to include it literally.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct HierarchyPath {
    components: Vec<PathComponent>,
}

impl HierarchyPath {
    /// Parses the accepted text forms described on [`HierarchyPath`].
    ///
    /// Returns [`PathError`] with a byte offset for invalid syntax.
    pub fn parse(text: &str) -> std::result::Result<Self, PathError> {
        path::parse(text, false).map(|selector| selector.path)
    }

    /// Builds a path from exact hierarchy components.
    ///
    /// String inputs are literal names. Use [`PathComponent::indexed`] for
    /// an array element.
    pub fn from_components<I, C>(components: I) -> Self
    where
        I: IntoIterator<Item = C>,
        C: Into<PathComponent>,
    {
        Self {
            components: components.into_iter().map(Into::into).collect(),
        }
    }

    /// Returns the exact hierarchy components in order.
    pub fn components(&self) -> impl ExactSizeIterator<Item = &PathComponent> + '_ {
        self.components.iter()
    }

    /// Returns the number of hierarchy components.
    pub fn len(&self) -> usize {
        self.components.len()
    }

    /// Returns whether the path has no components.
    pub fn is_empty(&self) -> bool {
        self.components.is_empty()
    }

    /// Returns the logical name of the final component, if any.
    pub fn name(&self) -> Option<&str> {
        self.components.last().map(PathComponent::name)
    }

    /// Returns the path without its final component, if any.
    pub fn parent(&self) -> Option<Self> {
        self.components.split_last().map(|(_, parent)| Self {
            components: parent.to_vec(),
        })
    }

    /// Returns a new path with one exact component appended.
    ///
    /// A string supplies a literal name, including its brackets.
    pub fn join(&self, component: impl Into<PathComponent>) -> Self {
        let mut path = self.clone();
        path.components.push(component.into());
        path
    }

    /// Formats the path in the selected style. Parsing the output restores the
    /// same path.
    ///
    /// [`PathStyle::Logical`] matches [`Display`](fmt::Display) and supports every
    /// name. [`PathStyle::Sv`] writes simple names directly and escapes other
    /// names. It also retains explicit escapes from input text or reader metadata.
    /// It ends each escaped name with an ASCII space, including at the end of a
    /// path. An array index follows the name and any terminating space.
    ///
    /// # Errors
    ///
    /// Returns [`PathFormatError::NotRepresentable`] for [`PathStyle::Sv`] if a
    /// name is empty or contains a character outside ASCII `0x21..=0x7e`.
    ///
    /// # Examples
    ///
    /// ```
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// use ondas::{HierarchyPath, PathStyle};
    ///
    /// let literal = HierarchyPath::parse(r#"top."u_reg[0]".q"#)?;
    /// assert_eq!(literal, HierarchyPath::parse(r"top.\u_reg[0] .q")?);
    /// assert_eq!(literal.format(PathStyle::Logical)?, r#"top."u_reg[0]".q"#);
    /// assert_eq!(literal.format(PathStyle::Sv)?, r"top.\u_reg[0] .q");
    ///
    /// let indexed = HierarchyPath::parse("top.u_reg[0].q")?;
    /// assert_ne!(indexed, literal);
    /// assert_eq!(indexed.format(PathStyle::Sv)?, "top.u_reg[0].q");
    /// # Ok(())
    /// # }
    /// ```
    pub fn format(&self, style: PathStyle) -> std::result::Result<String, PathFormatError> {
        match style {
            PathStyle::Logical => Ok(self.to_string()),
            PathStyle::Sv => path::to_verilog(&self.components),
        }
    }

    /// Formats the path with [`PathStyle::Sv`].
    ///
    /// See [`Self::format`] for the spelling rules and errors.
    pub fn to_verilog(&self) -> std::result::Result<String, PathFormatError> {
        self.format(PathStyle::Sv)
    }
}

impl FromStr for HierarchyPath {
    type Err = PathError;

    /// Parses an exact hierarchy path.
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
            let name = component.name();
            if path::simple_name(name) {
                formatter.write_str(name)?;
            } else {
                formatter.write_str(&serde_json::to_string(name).map_err(|_| fmt::Error)?)?;
            }
            if let Some(index) = component.index() {
                write!(formatter, "[{index}]")?;
            }
        }
        Ok(())
    }
}

/// An owned signal path and optional explicit bit projection.
///
/// The path can be relative to a scope for [`Hierarchy::resolve_selector`].
/// Projection bounds are inclusive positions in the whole signal.
/// Position zero is the least-significant bit.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SignalSelector {
    path: HierarchyPath,
    projection: Option<(u32, u32)>,
}

impl SignalSelector {
    /// Creates a selector from an exact path and optional `(msb, lsb)` bounds.
    ///
    /// Resolution checks the order of the bounds and the signal width.
    pub fn new(path: HierarchyPath, projection: Option<(u32, u32)>) -> Self {
        Self { path, projection }
    }

    /// Parses a hierarchy path and an optional final `[msb:lsb]` projection.
    ///
    /// The bounds are unsigned decimal `u32` values. Brackets inside a quoted
    /// or SystemVerilog escaped name are part of the name. An array index can
    /// precede the projection, as in `top.mem[-1][3:0]`.
    /// Invalid syntax returns [`PathError`] with a byte offset.
    pub fn parse(text: &str) -> std::result::Result<Self, PathError> {
        path::parse(text, true)
    }

    /// Returns the path without the projection.
    pub fn path(&self) -> &HierarchyPath {
        &self.path
    }

    /// Returns the explicit inclusive `(msb, lsb)` projection, if present.
    pub fn projection(&self) -> Option<(u32, u32)> {
        self.projection
    }

    /// Formats the path in the selected style and appends any projection.
    ///
    /// See [`HierarchyPath::format`] for the spelling rules and errors.
    pub fn format(&self, style: PathStyle) -> std::result::Result<String, PathFormatError> {
        let mut output = self.path.format(style)?;
        if let Some((msb, lsb)) = self.projection {
            write!(output, "[{msb}:{lsb}]").expect("writing to a String succeeds");
        }
        Ok(output)
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
/// VCD split declarations can form one normalized variable and signal. Their
/// private operand histories are not listed unless another declaration uses them.
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
    // Identity of this logical source. Clones keep it, so views from a clone compare equal.
    source: u64,
    data: Arc<HierarchyData>,
}

pub(crate) struct ScopeData {
    pub(crate) component: PathComponent,
    pub(crate) name_was_escaped: bool,
    pub(crate) parent: Option<usize>,
    pub(crate) kind: String,
    pub(crate) definition_name: Option<String>,
    pub(crate) packing: Option<Packing>,
    pub(crate) is_hidden: bool,
}

pub(crate) struct VariableData {
    pub(crate) component: PathComponent,
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
    // Whole-width logic keys are validated and canonicalized by for_encoding.
    variants: Vec<(String, String)>,
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
    scopes: Vec<ScopeData>,
    variables: Vec<VariableData>,
    encodings: Vec<Encoding>,
    // Imported definitions follow reader encodings. Their operands are reader indices.
    compositions: Vec<Box<[usize]>>,
    public_signals: Option<Vec<usize>>,
    children: OnceLock<ChildLinks>,
    declarations_by_name: OnceLock<NameIndex>,
}

struct NameIndex {
    offsets: Vec<usize>,
    declarations: Vec<usize>,
}

struct ChildLinks {
    first: Vec<usize>,
    next: Vec<usize>,
}

impl ChildLinks {
    fn new(scopes: &[ScopeData], variables: &[VariableData]) -> Self {
        let mut first = vec![usize::MAX; scopes.len()];
        let mut next = vec![usize::MAX; scopes.len() + variables.len()];
        // Scope IDs precede variable IDs.
        // Prepending in reverse order keeps scopes before variables and retains
        // declaration order within each kind, without per-parent allocations.
        for (index, variable) in variables.iter().enumerate().rev() {
            if let Some(parent) = variable.parent {
                let child = scopes.len() + index;
                next[child] = std::mem::replace(&mut first[parent], child);
            }
        }
        for (index, scope) in scopes.iter().enumerate().rev() {
            if let Some(parent) = scope.parent {
                next[index] = std::mem::replace(&mut first[parent], index);
            }
        }
        Self { first, next }
    }
}

/// Retained bytes of one vector payload, counted by capacity rather than length.
fn vec_bytes<T>(capacity: usize) -> usize {
    capacity.saturating_mul(std::mem::size_of::<T>())
}

static NEXT_SOURCE: AtomicU64 = AtomicU64::new(1);

fn next_source() -> u64 {
    NEXT_SOURCE
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
        .expect("hierarchy source IDs exhausted")
}

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

/// A selected declaration, its query handle, and an optional explicit bit slice.
///
/// The declaration preserves the selected alias and its original metadata.
/// Equal signal handles do not imply equal declarations or equal projections.
/// This view borrows only the hierarchy, not the selector text or scope path.
pub struct ResolvedSelector<'h> {
    declaration: Variable<'h>,
    signal: Signal,
    projection: Option<(u32, u32)>,
}

impl<'h> ResolvedSelector<'h> {
    /// Returns the selected declaration, including its original metadata.
    ///
    /// A projection does not change the declaration's signedness, logic domain,
    /// enumeration or HDL range. The caller chooses how to interpret projected bits.
    pub fn declaration(&self) -> Variable<'h> {
        Variable {
            hierarchy: self.declaration.hierarchy,
            index: self.declaration.index,
        }
    }

    /// Returns the resolved query handle, with the requested slice applied.
    pub fn signal(&self) -> Signal {
        self.signal
    }

    /// Returns the explicit inclusive normalized `(msb, lsb)` slice, if selected.
    ///
    /// Position zero is the rightmost, least-significant bit of the whole signal.
    /// These are not HDL indices from [`Variable::range`]. A full-width slice
    /// still returns `Some`, even when its handle equals the whole signal.
    /// An exact declaration, including a literal name such as `data[7:0]`, returns `None`.
    pub fn projection(&self) -> Option<(u32, u32)> {
        self.projection
    }

    /// Returns the fully qualified path of the chosen declaration and its
    /// original explicit projection.
    pub fn selector(&self) -> SignalSelector {
        SignalSelector::new(self.declaration.path(), self.projection)
    }
}

impl PartialEq<Scope<'_>> for Scope<'_> {
    fn eq(&self, other: &Scope<'_>) -> bool {
        self.hierarchy.source == other.hierarchy.source && self.index == other.index
    }
}

impl Eq for Scope<'_> {}

impl PartialEq<Variable<'_>> for Variable<'_> {
    fn eq(&self, other: &Variable<'_>) -> bool {
        self.hierarchy.source == other.hierarchy.source && self.index == other.index
    }
}

impl Eq for Variable<'_> {}

/// An opaque source or computed waveform signal with an optional bit projection.
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
/// [`Waveform::concat`](crate::Waveform::concat) creates computed bit signals.
/// Identical ordered definitions in one waveform share a handle. A computed
/// definition does not create declarations. Imported VCD definitions can have
/// normalized declarations, also when `concat` reuses their handles. The
/// definition belongs to the waveform;
/// copying a handle does not keep that waveform alive.
///
/// A projection has its own observed history. Only changes to the selected bits
/// appear in traces and scans. For projected samples, `changed_at` identifies the
/// last change of the projected value, not unrelated activity in its base signal.
/// It is `None` if the backend cannot establish that time reliably.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Signal {
    source: u64,
    index: usize,
    computed: bool,
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
        Self::with_compositions(scopes, variables, encodings, Vec::new())
    }

    pub(crate) fn with_compositions(
        scopes: Vec<ScopeData>,
        variables: Vec<VariableData>,
        mut encodings: Vec<Encoding>,
        compositions: Vec<Box<[usize]>>,
    ) -> Self {
        let public_signals = if compositions.is_empty() {
            None
        } else {
            for parts in &compositions {
                let width = parts
                    .iter()
                    .map(|&part| match encodings[part] {
                        Encoding::Bits { width } => width,
                        _ => unreachable!("validated bit composition"),
                    })
                    .try_fold(0u32, u32::checked_add)
                    .expect("validated composition width");
                encodings.push(Encoding::Bits { width });
            }
            let mut visible = vec![false; encodings.len()];
            for variable in &variables {
                if let Some(index) = variable.signal {
                    visible[index] = true;
                }
            }
            Some(
                visible
                    .into_iter()
                    .enumerate()
                    .filter_map(|(index, visible)| visible.then_some(index))
                    .collect(),
            )
        };
        Self {
            source: next_source(),
            data: Arc::new(HierarchyData {
                scopes,
                variables,
                encodings,
                compositions,
                public_signals,
                children: OnceLock::new(),
                declarations_by_name: OnceLock::new(),
            }),
        }
    }

    // Bytes this hierarchy retains, including the lookup tables that are built on first use.
    // A reader that parks validated declarations needs a byte bound, and the lazy tables are a
    // deterministic function of the declarations, so they are counted before they exist.
    pub(crate) fn validated_bytes(&self) -> usize {
        let data = &self.data;
        let scopes = data.scopes.len();
        let variables = data.variables.len();
        let mut bytes = std::mem::size_of::<HierarchyData>();
        bytes = bytes.saturating_add(vec_bytes::<ScopeData>(data.scopes.capacity()));
        for scope in &data.scopes {
            bytes = bytes.saturating_add(scope.component.name.capacity());
            bytes = bytes.saturating_add(scope.kind.capacity());
            bytes = bytes.saturating_add(
                scope
                    .definition_name
                    .as_ref()
                    .map_or(0, |name| name.capacity()),
            );
        }
        bytes = bytes.saturating_add(vec_bytes::<VariableData>(data.variables.capacity()));
        for variable in &data.variables {
            bytes = bytes.saturating_add(variable.component.name.capacity());
            bytes = bytes.saturating_add(variable.kind.capacity());
            bytes = bytes.saturating_add(
                variable
                    .reader_name
                    .as_ref()
                    .map_or(0, |name| name.capacity()),
            );
            bytes = bytes.saturating_add(
                variable
                    .type_name
                    .as_ref()
                    .map_or(0, |name| name.capacity()),
            );
            if let Some(enumeration) = &variable.enumeration {
                bytes = bytes.saturating_add(std::mem::size_of::<EnumerationData>());
                bytes = bytes
                    .saturating_add(enumeration.name.as_ref().map_or(0, |name| name.capacity()));
                bytes = bytes.saturating_add(vec_bytes::<(String, String)>(
                    enumeration.variants.capacity(),
                ));
                for (key, label) in &enumeration.variants {
                    bytes = bytes.saturating_add(key.capacity());
                    bytes = bytes.saturating_add(label.capacity());
                }
            }
        }
        bytes = bytes.saturating_add(vec_bytes::<Encoding>(data.encodings.capacity()));
        bytes = bytes.saturating_add(vec_bytes::<Box<[usize]>>(data.compositions.capacity()));
        for parts in &data.compositions {
            bytes = bytes.saturating_add(vec_bytes::<usize>(parts.len()));
        }
        if let Some(signals) = &data.public_signals {
            bytes = bytes.saturating_add(vec_bytes::<usize>(signals.capacity()));
        }
        // Children links keep one entry per scope and one per declaration; the name index keeps
        // one offset per scope plus two, and one declaration index per declaration.
        let child_entries = scopes.saturating_mul(2).saturating_add(variables);
        let name_entries = scopes
            .saturating_mul(2)
            .saturating_add(variables)
            .saturating_add(2);
        let words = std::mem::size_of::<usize>();
        bytes = bytes.saturating_add(child_entries.saturating_mul(words));
        bytes = bytes.saturating_add(name_entries.saturating_mul(words));
        bytes
    }

    // A reader may retain immutable declaration data across sequential logical opens. Every
    // open still needs its own source identity so handles from an earlier open are rejected.
    pub(crate) fn reopened(&self) -> Self {
        Self {
            source: next_source(),
            data: Arc::clone(&self.data),
        }
    }

    pub(crate) fn signal_at(&self, index: usize) -> Signal {
        let encoding = self.data.encodings[index];
        let sources = self.data.encodings.len() - self.data.compositions.len();
        if index >= sources {
            let Encoding::Bits { width } = encoding else {
                unreachable!()
            };
            return self.computed_signal(index - sources, width);
        }
        Signal {
            source: self.source,
            index,
            computed: false,
            base_encoding: encoding,
            encoding,
            lsb: 0,
        }
    }

    pub(crate) fn imported_compositions(&self) -> impl Iterator<Item = (u32, Vec<Signal>)> + '_ {
        let sources = self.data.encodings.len() - self.data.compositions.len();
        self.data
            .compositions
            .iter()
            .enumerate()
            .map(move |(index, parts)| {
                let width = self
                    .signal_at(sources + index)
                    .width()
                    .expect("imported bits");
                (
                    width,
                    parts.iter().map(|&index| self.signal_at(index)).collect(),
                )
            })
    }

    pub(crate) fn validate(&self, signal: Signal) -> Result<usize> {
        let sources = self.data.encodings.len() - self.data.compositions.len();
        let encoding = (!signal.computed && signal.index < sources)
            .then(|| self.data.encodings.get(signal.index).copied())
            .flatten();
        self.validate_encoding(signal, encoding)
    }

    pub(crate) fn computed_signal(&self, index: usize, width: u32) -> Signal {
        let encoding = Encoding::Bits { width };
        Signal {
            source: self.source,
            index,
            computed: true,
            base_encoding: encoding,
            encoding,
            lsb: 0,
        }
    }

    pub(crate) fn validate_encoding(
        &self,
        signal: Signal,
        encoding: Option<Encoding>,
    ) -> Result<usize> {
        let valid = signal.source == self.source
            && encoding == Some(signal.base_encoding)
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

    fn children_of(&self, parent: usize) -> impl Iterator<Item = Item<'_>> + '_ {
        let links = self
            .data
            .children
            .get_or_init(|| ChildLinks::new(&self.data.scopes, &self.data.variables));
        let scopes = self.data.scopes.len();
        let mut next = links.first[parent];
        std::iter::from_fn(move || {
            let index = next;
            if index == usize::MAX {
                return None;
            }
            next = links.next[index];
            Some(if index < scopes {
                Item::Scope(Scope {
                    hierarchy: self,
                    index,
                })
            } else {
                Item::Variable(Variable {
                    hierarchy: self,
                    index: index - scopes,
                })
            })
        })
    }

    fn scope_path_at(&self, mut index: Option<usize>) -> HierarchyPath {
        let mut components = Vec::new();
        while let Some(current) = index {
            let scope = &self.data.scopes[current];
            components.push(scope.component.clone().with_escape(scope.name_was_escaped));
            index = scope.parent;
        }
        components.reverse();
        HierarchyPath { components }
    }

    /// Iterates over root scopes and variables.
    pub fn roots(&self) -> impl Iterator<Item = Item<'_>> + '_ {
        self.scopes()
            .filter(|scope| scope.data().parent.is_none())
            .map(Item::Scope)
            .chain(
                self.variables()
                    .filter(|variable| variable.data().parent.is_none())
                    .map(Item::Variable),
            )
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
    /// Private operands of normalized VCD signals are omitted unless they have
    /// a separate public declaration.
    pub fn signals(&self) -> impl Iterator<Item = Signal> + '_ {
        let count = self
            .data
            .public_signals
            .as_ref()
            .map_or(self.data.encodings.len(), Vec::len);
        (0..count).map(|index| {
            let slot = self
                .data
                .public_signals
                .as_ref()
                .map_or(index, |signals| signals[index]);
            self.signal_at(slot)
        })
    }

    /// Parses and resolves an exact scope path.
    pub fn scope(&self, text: &str) -> std::result::Result<Scope<'_>, LookupError> {
        self.scope_path(&HierarchyPath::parse(text)?)
    }

    /// Parses and resolves an exact variable path.
    pub fn variable(&self, text: &str) -> std::result::Result<Variable<'_>, LookupError> {
        self.variable_path(&HierarchyPath::parse(text)?)
    }

    /// Resolves a [`SignalSelector`] from text and returns its query handle.
    ///
    /// `tb.data[3:0]` slices `data`. To select a literal `data[3:0]`,
    /// use `tb."data[3:0]"` or `tb.\data[3:0]␠`, where `␠` is the required
    /// trailing ASCII space.
    /// Use [`Self::resolve_signal`] to retain the declaration and explicit slice.
    ///
    /// Returns [`LookupError`] for invalid syntax, missing or ambiguous metadata,
    /// a declaration without a history, or an invalid slice.
    pub fn signal(&self, selector: &str) -> std::result::Result<Signal, LookupError> {
        self.resolve_signal(selector, None)
            .map(|resolved| resolved.signal())
    }

    /// Parses a [`SignalSelector`] and resolves it with [`Self::resolve_selector`].
    ///
    /// Syntax error offsets refer to bytes in the supplied text.
    pub fn resolve_signal(
        &self,
        selector: &str,
        scope: Option<&HierarchyPath>,
    ) -> std::result::Result<ResolvedSelector<'_>, LookupError> {
        self.resolve_selector(&SignalSelector::parse(selector)?, scope)
    }

    /// Resolves a parsed selector and retains its declaration and explicit bit slice.
    ///
    /// `scope` is a root-based component prefix, not a scope identity. An empty
    /// prefix or `None` selects from the root. A candidate that starts with the
    /// prefix and has at least one further component is already qualified.
    /// Otherwise, the prefix is added once. An empty path returns
    /// [`LookupError::NotFound`] with an empty path. Duplicate scope paths can
    /// still resolve to a unique declaration.
    ///
    /// Lookup error paths identify the qualified base. If a unique declaration
    /// has a signal, resolution applies the projection with [`Signal::slice`].
    /// A full-width slice retains its bounds in [`ResolvedSelector::projection`].
    ///
    /// ```
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let source = b"$scope module top $end $var wire 1 ! flag $end \
    ///     $upscope $end $enddefinitions $end #0 1!";
    /// let mut wave = ondas::open_bytes("tree.vcd", source.as_slice().into())?;
    /// let h = wave.hierarchy().clone();
    /// let scope = ondas::HierarchyPath::parse("top")?;
    /// let whole = h.resolve_signal("flag", Some(&scope))?;
    /// let selector = ondas::SignalSelector::parse(r#""flag"[0:0]"#)?;
    /// assert_eq!(selector.path().len(), 1);
    /// let full = h.resolve_selector(&selector, Some(&scope))?;
    /// assert_eq!(whole.signal(), full.signal());
    /// assert_eq!(whole.projection(), None);
    /// assert_eq!(full.projection(), Some((0, 0)));
    /// assert_eq!(full.selector().format(ondas::PathStyle::Logical)?, "top.flag[0:0]");
    /// let sample = wave.sample(full.signal(), ondas::Time::ZERO)?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn resolve_selector(
        &self,
        selector: &SignalSelector,
        scope: Option<&HierarchyPath>,
    ) -> std::result::Result<ResolvedSelector<'_>, LookupError> {
        let path = selector.path();
        let qualified;
        let path = if let Some(scope) = scope.filter(|scope| !scope.is_empty())
            && !path.is_empty()
            && !(path.len() > scope.len() && path.components.starts_with(&scope.components))
        {
            qualified = HierarchyPath::from_components(scope.components().chain(path.components()));
            &qualified
        } else {
            path
        };
        let declaration = self.variable_path(path)?;
        let signal = declaration
            .signal()
            .ok_or_else(|| LookupError::NoSignal { path: path.clone() })?;
        let projection = selector.projection();
        let signal = match projection {
            Some((msb, lsb)) => signal.slice(msb, lsb)?,
            None => signal,
        };
        Ok(ResolvedSelector {
            declaration,
            signal,
            projection,
        })
    }

    /// Resolves an already parsed exact scope path.
    ///
    /// Returns [`LookupError::Ambiguous`] when distinct scopes share the path.
    /// Those scopes remain accessible through [`Self::scopes`] and [`Scope::children`].
    pub fn scope_path(&self, path: &HierarchyPath) -> std::result::Result<Scope<'_>, LookupError> {
        let mut matches = self.scopes().filter(|scope| {
            Some(&scope.data().component) == path.components.last() && scope.path() == *path
        });
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
        let Some((name, scopes)) = path.components.split_last() else {
            return Err(LookupError::NotFound { path: path.clone() });
        };
        // The common warm path has one scope candidate per component. Follow it
        // without building a frontier vector. A second candidate can still hold
        // the target in another branch, so hand that case to the full frontier
        // search instead of reporting an early ambiguity.
        let mut parent = None;
        for name in scopes {
            let mut candidates = self
                .declarations_named(parent, name)
                .iter()
                .copied()
                .filter(|index| *index < self.data.scopes.len());
            let Some(first) = candidates.next() else {
                return Err(LookupError::NotFound { path: path.clone() });
            };
            if candidates.next().is_some() {
                return self.variable_path_frontier(path);
            }
            parent = Some(first);
        }
        let mut matches = self
            .declarations_named(parent, name)
            .iter()
            .copied()
            .filter_map(|index| index.checked_sub(self.data.scopes.len()))
            .map(|index| Variable {
                hierarchy: self,
                index,
            });
        let first = matches.next();
        match (first, matches.count()) {
            (None, _) => Err(LookupError::NotFound { path: path.clone() }),
            (Some(variable), 0) => Ok(variable),
            (_, remaining) => Err(LookupError::Ambiguous {
                path: path.clone(),
                matches: remaining + 1,
            }),
        }
    }

    /// Complete frontier search for a path with several scope branches.
    fn variable_path_frontier(
        &self,
        path: &HierarchyPath,
    ) -> std::result::Result<Variable<'_>, LookupError> {
        let Some((name, scopes)) = path.components.split_last() else {
            return Err(LookupError::NotFound { path: path.clone() });
        };
        let mut parents = vec![None];
        for name in scopes {
            parents = parents
                .into_iter()
                .flat_map(|parent| self.declarations_named(parent, name))
                .copied()
                .filter(|index| *index < self.data.scopes.len())
                .map(Some)
                .collect();
            if parents.is_empty() {
                return Err(LookupError::NotFound { path: path.clone() });
            }
        }
        let mut matches = parents
            .into_iter()
            .flat_map(|parent| self.declarations_named(parent, name))
            .filter_map(|index| index.checked_sub(self.data.scopes.len()))
            .map(|index| Variable {
                hierarchy: self,
                index,
            });
        let first = matches.next();
        match (first, matches.count()) {
            (None, _) => Err(LookupError::NotFound { path: path.clone() }),
            (Some(variable), 0) => Ok(variable),
            (_, remaining) => Err(LookupError::Ambiguous {
                path: path.clone(),
                matches: remaining + 1,
            }),
        }
    }

    fn declaration_key(&self, index: usize) -> (Option<usize>, &PathComponent) {
        if index < self.data.scopes.len() {
            let scope = &self.data.scopes[index];
            (scope.parent, &scope.component)
        } else {
            let variable = &self.data.variables[index - self.data.scopes.len()];
            (variable.parent, &variable.component)
        }
    }

    fn declarations_named(&self, parent: Option<usize>, name: &PathComponent) -> &[usize] {
        // Keep only declaration indices. Full paths duplicate every ancestor's
        // name for each variable and are unnecessary for component lookup.
        let index = self.data.declarations_by_name.get_or_init(|| {
            let root = self.data.scopes.len();
            let count = root + self.data.variables.len();
            let mut offsets = vec![0; root + 2];
            for item in 0..count {
                let parent = self.declaration_key(item).0.unwrap_or(root);
                offsets[parent + 1] += 1;
            }
            for parent in 1..offsets.len() {
                offsets[parent] += offsets[parent - 1];
            }
            let mut next = offsets[..offsets.len() - 1].to_vec();
            let mut declarations = vec![0; count];
            for item in 0..count {
                let parent = self.declaration_key(item).0.unwrap_or(root);
                declarations[next[parent]] = item;
                next[parent] += 1;
            }
            for range in offsets.windows(2) {
                declarations[range[0]..range[1]].sort_unstable_by(|a, b| {
                    self.declaration_key(*a).1.cmp(self.declaration_key(*b).1)
                });
            }
            NameIndex {
                offsets,
                declarations,
            }
        });
        let parent = parent.unwrap_or(self.data.scopes.len());
        let index = &index.declarations[index.offsets[parent]..index.offsets[parent + 1]];
        let start = index.partition_point(|item| self.declaration_key(*item).1 < name);
        let count = index[start..].partition_point(|item| self.declaration_key(*item).1 == name);
        &index[start..start + count]
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
    /// Imported VCD compositions have their normalized declarations. A computed
    /// definition without declarations returns an empty iterator. Compositions
    /// do not inherit aliases from their operands.
    ///
    /// A handle from another hierarchy or waveform produces
    /// [`Error::InvalidSignal`](crate::Error::InvalidSignal).
    pub fn aliases(&self, signal: Signal) -> Result<impl Iterator<Item = Variable<'_>> + '_> {
        if signal.computed {
            self.validate_encoding(signal, Some(signal.base_encoding))?;
        }
        if !signal.computed {
            self.validate(signal)?;
        }
        Ok(self
            .variables()
            .filter(move |variable| variable.signal() == Some(signal.base())))
    }
}

impl<'h> Scope<'h> {
    fn data(&self) -> &'h ScopeData {
        &self.hierarchy.data.scopes[self.index]
    }

    /// Returns the scope's logical local name, without the array index.
    pub fn name(&self) -> &'h str {
        self.data().component.name()
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
        self.hierarchy.children_of(self.index)
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

    /// Returns the declaration's logical local name, without the array index.
    ///
    /// Recognized Verilog escape syntax is excluded. Literal backslashes,
    /// brackets, dots and whitespace remain name data. Use [`Self::path`] for
    /// lookup.
    pub fn name(&self) -> &'h str {
        self.data().component.name()
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
        let data = self.data();
        let mut path = self.hierarchy.scope_path_at(data.parent);
        path.components
            .push(data.component.clone().with_escape(data.name_was_escaped));
        path
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
    ///
    /// A normalized VCD split group returns `None`: its recorded coverage does
    /// not establish the original whole HDL range. Use [`Signal::width`] and
    /// [`Signal::slice`] for normalized positions.
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

    /// Selects an inclusive HDL index range from a bit-vector signal.
    ///
    /// This range must have the signal's current width. Its right endpoint maps
    /// to normalized position zero. Requested indices must lie inside the range
    /// and follow its direction. This also accepts a caller-defined range for a
    /// computed value; it does not verify the range's declaration provenance.
    /// Use [`Variable::range`] for source declaration indices. Absent metadata
    /// does not imply a range. [`Signal::slice`] always uses normalized positions.
    ///
    /// Width, bounds and direction failures return distinct [`SliceError`]
    /// variants. Extreme endpoints return an error without arithmetic overflow.
    /// Handle ownership is checked when the result is used with a waveform.
    ///
    /// ```
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let input = b"$var wire 8 ! data [0:7] $end $enddefinitions $end #0 b10110010 !";
    /// let wave = ondas::open_bytes("range.vcd", input.as_slice().into())?;
    /// let variable = wave.hierarchy().variable("data")?;
    /// let signal = variable.signal().unwrap();
    /// assert_eq!(variable.range().unwrap().slice(signal, 2, 5)?, signal.slice(5, 2)?);
    /// # Ok(())
    /// # }
    /// ```
    pub fn slice(
        self,
        signal: Signal,
        msb: i64,
        lsb: i64,
    ) -> std::result::Result<Signal, SliceError> {
        let width = signal.width().ok_or(SliceError::NotBits)?;
        if i128::from(self.msb).abs_diff(i128::from(self.lsb)) + 1 != u128::from(width) {
            return Err(SliceError::HdlRangeWidthMismatch { range: self, width });
        }
        let low = self.msb.min(self.lsb);
        let high = self.msb.max(self.lsb);
        if !(low..=high).contains(&msb) || !(low..=high).contains(&lsb) {
            return Err(SliceError::HdlOutOfBounds {
                range: self,
                msb,
                lsb,
            });
        }
        let position = |index: i64| index.abs_diff(self.lsb) as u32;
        let (left, right) = (position(msb), position(lsb));
        if left < right {
            return Err(SliceError::HdlDirectionMismatch {
                range: self,
                msb,
                lsb,
            });
        }
        signal.slice(left, right)
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
/// Compare keys with queried [`Bits`](crate::Bits) or [`BitsRef`](crate::BitsRef)
/// values using `==`. Equality checks the complete width and every logic state
/// without formatting text or copying the bit payload.
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
///             if variant.encoded == bits {
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
    pub(crate) fn is_computed(self) -> bool {
        self.computed
    }
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
