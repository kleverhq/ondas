#include "shm.h"
#include "shm_sst2.h"
#include "shm_value.h"
#include <algorithm>
#include <cstdio>
#include <cstring>
#include <filesystem>
#include <memory>
#include <limits>
#include <stdexcept>
#include <string>
#include <unordered_map>
#include <unordered_set>
#include <vector>

namespace {
struct Unavailable : std::runtime_error { using std::runtime_error::runtime_error; };
void require(bool ok, const char *message) { if (!ok) throw std::runtime_error(message); }
void report(char *out, size_t cap, const char *message) noexcept {
    if (cap) std::snprintf(out, cap, "%s", message);
}
int filesystem_failure(const std::filesystem::filesystem_error &failure, int *os_error,
                       char *out, size_t cap) noexcept {
    report(out, cap, failure.what());
    const auto &code = failure.code();
    if (code.value() && (code.category() == std::system_category() || code.category() == std::generic_category())) {
        *os_error = code.value();
        return -2;
    }
    return -1;
}
std::string text(int property, SstHandle handle) {
    if (!handle) return {};
    const char *value = da_get_sstr(property, handle);
    return value ? value : "";
}
void check_handle(SstHandle handle, const char *operation) {
    if (handle && da_get(0x12, handle) > 0) {
        std::string error = text(0x1a, handle);
        da_delete(handle);
        throw std::runtime_error(error.empty() ? operation : std::string(operation) + ": " + error);
    }
}
struct Enumeration {
    SstHandle handle;
    Enumeration(int property, SstHandle scope) : handle(da_iterate(property, scope)) {
        check_handle(handle, "cannot enumerate SHM hierarchy");
    }
    SstHandle next() {
        if (!handle) return 0;
        SstHandle object = da_scan(handle);
        // da_scan destroys an exhausted iterator, including empty enumerations.
        if (!object) handle = 0;
        // Ordinary declarations belong to the Session. An error result is a
        // separate owned object; check_handle deletes only that error class.
        else check_handle(object, "cannot scan SHM hierarchy");
        return object;
    }
    ~Enumeration() { if (handle) da_delete(handle); }
};
void suspend_active();
struct Session {
    SstHandle handle = 0;
    explicit Session(const std::string &path) {
        suspend_active();
        static const bool initialized = sst2Init() == 1;
        if (!initialized) throw Unavailable("sst2Init failed");
        try {
            std::string stem;
            for (const auto &entry : std::filesystem::directory_iterator(path)) {
                if (entry.path().extension() == ".dsn") {
                    require(stem.empty(), "SHM directory has multiple DSN components");
                    stem = entry.path().stem().string();
                }
            }
            require(!stem.empty(), "SHM directory has no DSN component");
            std::vector<std::pair<std::uint64_t, std::string>> components;
            for (const auto &entry : std::filesystem::directory_iterator(path)) {
                if (entry.path().extension() != ".trn") continue;
                const auto name = entry.path().stem().string();
                std::uint64_t number = 0;
                if (name != stem) {
                    require(name.compare(0, stem.size() + 1, stem + "-") == 0,
                            "SHM TRN component has an unexpected name");
                    const auto suffix = name.substr(stem.size() + 1);
                    require(!suffix.empty() && suffix.find_first_not_of("0123456789") == std::string::npos,
                            "SHM TRN component has an invalid numeric suffix");
                    number = std::stoull(suffix);
                    require(number > 0, "SHM TRN component has a zero suffix");
                }
                components.emplace_back(number, entry.path().string());
            }
            std::sort(components.begin(), components.end());
            require(!components.empty() && components.front().first == 0,
                    "SHM directory has no base TRN component");
            for (std::size_t i = 0; i < components.size(); ++i) {
                require(i == 0 || components[i - 1].first != components[i].first,
                        "SHM TRN components have duplicate numeric suffixes");
                SstHandle component = 0;
                int status = drOpenFile(components[i].second.c_str(), &component, nullptr, nullptr);
                if (i == 0) {
                    if (status == 0 && component) {
                        handle = component;
                    } else {
                        // A stale base can precede valid numbered components.
                        // Anchor the directory's DSN through the original path.
                        if (component) drClose(component);
                        component = 0;
                        const int directory_status = drOpenFile(path.c_str(), &handle, nullptr, nullptr);
                        if (directory_status != 0 || !handle)
                            throw std::runtime_error("cannot open SHM database directory (SDK status " + std::to_string(directory_status) + ")");
                        status = drOpenFile(components[i].second.c_str(), &component, nullptr, nullptr);
                    }
                }
                if (component && component != handle) drClose(component);
                // Repeated simulations can leave TRNs for a different design.
                // SST2 reports -140 (No matching design), as its own directory
                // reader does. These files are not part of the current DSN.
                if (status == -140 && !component) continue;
                if (status != 0 || component != handle)
                    throw std::runtime_error("cannot join SHM TRN component to the database (SDK status " + std::to_string(status) + ")");
            }
            // Ordinary SST2 opens share one file handle. Opening the base and
            // then every numeric component selects the latest recording and
            // joins its split intervals. Close the shared handle only once.
        } catch (...) {
            if (handle) drClose(handle);
            handle = 0;
            throw;
        }
    }
    Session(const Session &) = delete;
    Session &operator=(const Session &) = delete;
    ~Session() { if (handle) drClose(handle); }
};
constexpr size_t root_node = std::numeric_limits<size_t>::max();
struct SourceNode {
    size_t parent, ordinal;
    int property;
    std::string name;
};
size_t source_node(std::vector<SourceNode> &nodes, size_t parent, int property,
                   size_t ordinal, SstHandle object) {
    nodes.push_back({parent, ordinal, property, text(0x10a, object)});
    return nodes.size() - 1;
}
struct Declaration {
    ondas_shm_decl descriptor{};
    std::string name, kind, definition;
    std::string enum_name;
    std::vector<std::pair<std::string, std::string>> variants;
    size_t source = root_node;
    int type_kind = -1;
    SstHandle variable = 0; // only valid within this declaration's Session
};
std::int64_t index_left(SstHandle type) {
    SstHandle index = da_handle(0x1de, type);
    int width = 0;
    require(da_get_int(0x164, index, &width) >= 0 && (width == 32 || width == 64),
            "unsupported SHM declaration index representation");
    // The index type proves the fixed byte count. Its representation can
    // contain zero bytes, so the borrowed buffer is never measured with strlen.
    const char *bytes = da_get_sstr(0x11b, type);
    require(bytes != nullptr, "cannot read SHM declaration index");
    std::uint64_t value = 0;
    for (int i = 0; i < width / 8; ++i) value = (value << 8) | static_cast<unsigned char>(bytes[i]);
    return width == 32 ? static_cast<std::int32_t>(value) : static_cast<std::int64_t>(value);
}
Declaration describe_variable(SstHandle variable, const std::string &prefix) {
    Declaration declaration;
    auto &d = declaration.descriptor;
    declaration.variable = variable;
    declaration.name = prefix.empty() ? text(0x10a, variable) : prefix + "[" + text(0x10a, variable) + "]";
    declaration.kind = "shm:decl:" + std::to_string(da_get(0x143, variable));
    d.entry = 2;
    d.is_constant = da_get(0x122, variable) > 0;
    SstHandle type = da_handle(0x1be, variable);
    int kind = da_get(0x173, type);
    declaration.type_kind = kind;
    int width = 0;
    if ((kind == 2 || kind == 4) && da_get_int(0x3d4, type, &width) >= 0 && width > 0) {
        d.encoding = 1;
        d.width = static_cast<std::uint32_t>(width);
        SstHandle index = da_handle(0x1de, type);
        if (index && da_get(0x1b2, type) == 0) {
            int count = 0;
            require(da_get_int(0x184, type, &count) >= 0 && count > 0, "invalid SHM array size");
            // Signed integer storage is an internal array with no HDL range.
            if (da_get(0x192, type) > 0 || da_get(0x103, variable) == 7) {
                d.has_range = 1;
                d.msb = index_left(type);
                const std::int64_t offset = da_get(0x1a2, type) > 0 ? count - 1LL : 1LL - count;
                require(offset >= 0 ? d.msb <= std::numeric_limits<std::int64_t>::max() - offset
                                    : d.msb >= std::numeric_limits<std::int64_t>::min() - offset,
                        "SHM declaration index overflow");
                d.lsb = d.msb + offset;
            }
        }
    } else if (kind == 5) d.encoding = 2;
    else if (kind == 7) d.encoding = 3;
    else if (kind == 1) d.encoding = 4;
    return declaration;
}
void import_enumeration(Declaration &declaration) {
    const auto &d = declaration.descriptor;
    if (d.encoding != 1) return;
    const auto type = da_handle(0x1be, declaration.variable);
    Enumeration elements(0x1fe, type);
    auto element = elements.next();
    if (!element) return;
    int raw_bits = 0;
    if (da_get_int(0x164, type, &raw_bits) < 0 || raw_bits <= 0) return;
    const int bytes = raw_bits / 8 + (raw_bits % 8 != 0);
    std::vector<std::pair<std::string, std::string>> variants;
    do {
        const char *label = da_get_sstr(0x10a, element);
        if (!label) return;
        std::string owned_label(label);
        const char *raw = da_get_sstr(0x10b, element);
        if (!raw) return;
        // Raw storage can contain NUL and is not an HDL integer. The source
        // type proves its byte count and formats it like ordinary bit values.
        const std::string owned_raw(raw, bytes);
        const char *formatted = da_get_sstr_pl(0x15a, type, 0x10b,
                                             owned_raw.data(), bytes, 0x1b3, 1, 0x10);
        if (!formatted) return;
        std::string key(formatted);
        if (key.size() != d.width) return;
        for (char &bit : key) {
            if (bit >= 'A' && bit <= 'Z') bit += 'a' - 'A';
            if (!bit || !std::strchr("01xzuwlh-", bit)) return;
        }
        variants.emplace_back(std::move(key), std::move(owned_label));
    } while ((element = elements.next()));
    declaration.enum_name = text(0x10a, type);
    declaration.variants = std::move(variants);
}
void import_variable(SstHandle variable, const std::string &prefix, std::vector<Declaration> &out,
                     std::unordered_map<SstHandle, std::uint64_t> &identities,
                     std::vector<SourceNode> &nodes, size_t source) {
    Declaration declaration = describe_variable(variable, prefix);
    import_enumeration(declaration);
    declaration.source = source;
    auto &d = declaration.descriptor;
#ifdef ONDAS_SHM_USE_ALIAS_IDENTITY
    SstHandle key = da_get_xref(0x13d, variable);
    require(key != 0, "missing SHM history identity");
    auto identity = identities.emplace(key, out.size());
    d.id = identity.first->second;
    if (!identity.second) {
        const auto &other = out[d.id].descriptor;
        require(other.encoding == d.encoding && other.width == d.width, "incompatible SHM alias declaration");
    }
#else
    // Without reader identity metadata, each declaration has its own history.
    d.id = out.size();
    (void)identities;
#endif
    std::string child_prefix = declaration.name;
    const int kind = declaration.type_kind;
    out.push_back(std::move(declaration));
    if (kind == 8) {
        for (int property : {0x2ae, 0x2be}) {
            Enumeration children(property, variable);
            size_t ordinal = 0;
            for (SstHandle child; (child = children.next()); ++ordinal) {
                const auto node = source_node(nodes, source, property, ordinal, child);
                import_variable(child, child_prefix, out, identities, nodes, node);
            }
        }
    }

}
void hierarchy(SstHandle parent, std::vector<Declaration> &out,
               std::unordered_map<SstHandle, std::uint64_t> &identities,
               std::vector<SourceNode> &nodes, size_t parent_node = root_node,
               const std::string &scope_array = "") {
    Enumeration children(0x2ae, parent);
    size_t ordinal = 0;
    for (SstHandle child; (child = children.next()); ++ordinal) {
        const auto node = source_node(nodes, parent_node, 0x2ae, ordinal, child);
        if (da_get(0x173, da_handle(0x1be, child)) >= 0) {
            import_variable(child, "", out, identities, nodes, node);
            continue;
        }
        Declaration scope;
        scope.name = text(0x10a, child);
        int kind = da_get(0x113, child);
        if (kind == 10) { hierarchy(child, out, identities, nodes, node, scope.name); continue; }
        if (!scope_array.empty()) scope.name = scope_array + "[" + scope.name + "]";
        scope.kind = kind == 6 ? "module" : kind == 3 ? "architecture" : "shm:scope:" + std::to_string(kind);
        SstHandle definition = da_handle(0x14e, child);
        scope.definition = text(0x10a, definition);
        // SST2 leaves the top VHDL architecture instance name empty. Its
        // definition carries library.entity(architecture); the SHM top name
        // is entity(architecture), as independently reported by simvisdbutil.
        if (scope.name.empty() && kind == 3 && da_get(0x103, child) == 7) {
            const auto separator = scope.definition.find('.');
            require(separator != std::string::npos, "missing SHM VHDL top definition");
            scope.name = scope.definition.substr(separator + 1);
        }
        out.push_back(std::move(scope));
        hierarchy(child, out, identities, nodes, node);
        Declaration end;
        end.descriptor.entry = 1;
        out.push_back(std::move(end));
    }
    Enumeration variables(0x2be, parent);
    ordinal = 0;
    for (SstHandle variable; (variable = variables.next()); ++ordinal) {
        const auto node = source_node(nodes, parent_node, 0x2be, ordinal, variable);
        import_variable(variable, "", out, identities, nodes, node);
    }
}

using SourceChildren = std::unordered_map<size_t, std::vector<size_t>>;
void resolve_children(SstHandle parent, size_t parent_node, const std::vector<SourceNode> &nodes,
                      SourceChildren &children, std::unordered_map<size_t, SstHandle> &resolved,
                      ondas_shm_work &work) {
    const auto entry = children.find(parent_node);
    if (entry == children.end()) return;
    auto &wanted = entry->second;
    std::sort(wanted.begin(), wanted.end(), [&](size_t a, size_t b) {
        return nodes[a].property != nodes[b].property ? nodes[a].property < nodes[b].property
                                                     : nodes[a].ordinal < nodes[b].ordinal;
    });
    for (size_t first = 0; first < wanted.size();) {
        const int property = nodes[wanted[first]].property;
        Enumeration enumeration(property, parent);
        size_t ordinal = 0;
        do {
            const auto index = wanted[first++];
            const auto &node = nodes[index];
            SstHandle object = 0;
            do {
                object = enumeration.next();
                require(object != 0, "SHM hierarchy changed after opening");
                ++work.hierarchy_nodes;
            } while (ordinal++ < node.ordinal);
            require(text(0x10a, object) == node.name, "SHM hierarchy changed after opening");
            resolved.emplace(index, object);
            resolve_children(object, index, nodes, children, resolved, work);
        } while (first < wanted.size() && nodes[wanted[first]].property == property);
    }
}
std::vector<Declaration> selected_declarations(SstHandle file, const uint64_t *ids, size_t count,
                                               const std::vector<Declaration> &original,
                                               const std::vector<SourceNode> &nodes,
                                               ondas_shm_work &work) {
    // Source files are immutable while open. Keep owned enumeration routes,
    // resolve only their shared ancestors, and validate the selected shapes.
    // All resolved handles remain local to this operation's Session.
    SourceChildren children;
    std::unordered_set<size_t> wanted{root_node};
    for (size_t i = 0; i < count; ++i) {
        require(ids[i] < original.size() && original[ids[i]].descriptor.entry == 2 &&
                original[ids[i]].descriptor.id == ids[i], "invalid SHM history identity");
        size_t node = original[ids[i]].source;
        while (wanted.insert(node).second) {
            require(node < nodes.size(), "invalid SHM source route");
            children[nodes[node].parent].push_back(node);
            node = nodes[node].parent;
        }
    }
    std::unordered_map<size_t, SstHandle> resolved;
    resolve_children(file, root_node, nodes, children, resolved, work);
    std::vector<Declaration> declarations;
    declarations.reserve(count);
    for (size_t i = 0; i < count; ++i) {
        const auto &expected = original[ids[i]];
        auto current = describe_variable(resolved.at(expected.source), "");
        const auto &a = expected.descriptor;
        const auto &b = current.descriptor;
        require(a.encoding == b.encoding && a.width == b.width &&
                a.is_constant == b.is_constant && a.has_range == b.has_range &&
                (!a.has_range || (a.msb == b.msb && a.lsb == b.lsb)) &&
                expected.kind == current.kind && expected.type_kind == current.type_kind,
                "SHM declaration changed after opening");
        current.name = expected.name;
        current.descriptor.id = ids[i];
        declarations.push_back(std::move(current));
    }
    return declarations;
}

ondas_shm_meta metadata(SstHandle file) {
    SstTime first{}, last{};
    require(da_get_xvalseq(0x106, file, &first) >= 0 && da_get_xvalseq(0x116, file, &last) >= 0,
            "cannot read SHM time span");
    int exponent = 0;
    SstHandle unit = da_handle(0x1ae, SstHandle(1));
    require(da_get_int(0x194, unit, &exponent) >= 0, "cannot read SHM timescale");
    return {first.tick, last.tick, exponent};
}
struct Cursor {
    SstHandle variable, handle = 0;
    explicit Cursor(SstHandle var, const SstTime *end = nullptr) : variable(var) {
        SstHandle loaded = da_handle(0x12f, variable);
        check_handle(loaded, "cannot load SHM history");
        require(loaded == 1, "cannot load SHM history");
        handle = end ? da_handle_pl(0x22e, variable, 0x1b3, 1, 0x116, *end, 0x10)
                     : da_handle_pl(0x22e, variable, 0x1b3, 1, 0x10);
        try { check_handle(handle, "cannot create SHM value iterator"); require(handle, "null SHM value iterator"); }
        catch (...) {
            handle = 0;
            SstHandle released = da_handle(0x13f, variable);
            if (released != 1 && released) da_delete(released);
            throw;
        }
    }
    Cursor(const Cursor &) = delete;
    Cursor &operator=(const Cursor &) = delete;
    ~Cursor() {
        if (handle) da_delete(handle);
        SstHandle result = da_handle(0x13f, variable);
        if (result != 1 && result) da_delete(result);
    }
};
bool point_type(const Declaration &d) {
    // These exact four-state logic, wire and integer layouts have fixed-width
    // 0/1/x/z formatting. Do not admit other domains by masking property bits:
    // the equality expression also certifies the prefix's availability.
    if (d.descriptor.encoding != 1 || !d.descriptor.width || d.descriptor.width > 4096) return false;
    const auto type = da_handle(0x1be, d.variable);
    check_handle(type, "cannot read SHM point type");
    require(type != 0, "missing SHM point type");
    int properties = 0;
    if (da_get_int(0x1c4, type, &properties) < 0) return false;
    if (d.descriptor.width == 1)
        return da_get(0x173, type) == 2 && da_get(0x1b3, type) == 1 &&
               ((da_get(0x1c3, type) == 3 && properties == 0x4060a) ||
                (da_get(0x1c3, type) == 5 && properties == 0x7004060a));
    int raw_bits = 0;
    if (da_get(0x173, type) != 4 || da_get(0x1c3, type) != 12 ||
        !((properties == 0x10060c && da_get(0x1b3, type) == 4) ||
          (properties == 0x10061c && da_get(0x1b3, type) == 3)) ||
        da_get_int(0x164, type, &raw_bits) < 0 || raw_bits != int(d.descriptor.width * 2)) return false;
    const auto element = da_handle(0x1ce, type);
    check_handle(element, "cannot read SHM point element type");
    int width = 0, bits = 0, element_properties = 0;
    return element && da_get(0x173, element) == 2 && da_get(0x1c3, element) == 3 &&
           da_get_int(0x1c4, element, &element_properties) >= 0 && element_properties == 0x4060a &&
           da_get_int(0x3d4, element, &width) >= 0 && width == 1 &&
           da_get_int(0x164, element, &bits) >= 0 && bits == 2;
}
bool point_prefix(const Declaration &d, std::uint64_t end, std::uint64_t &first) {
    // Exact equality propagates negative source states. Comparing a guarded
    // source with itself coalesces only contiguous valid availability. The
    // expression is Session-owned; Cursor balances its Load and Unload.
    const auto expression = da_handle_pl(0x32f, SstHandle(0), 0x183, 0x2e,
                                         0x21e, d.variable, 0x21e, d.variable, 0x10);
    check_handle(expression, "cannot construct SHM point expression");
    require(expression != 0, "missing SHM point expression");
    const SstTime bound{end, UINT64_MAX};
    Cursor cursor(expression, &bound);
    SstTime current{}, next{};
    require(da_get_xvalseq(0x136, cursor.handle, &current) >= 0 &&
            da_get_xvalseq(0x146, cursor.handle, &next) >= 0, "cannot read SHM prefix bounds");
    if (current.tick != 0) return false;
    if (da_get(0x1a3, cursor.handle) < 0) {
        // An absent initial state is legal. Accept only one leading missing
        // interval followed by continuous availability through the bound.
        // A later missing interval still rejects the entire certificate.
        if (next.tick > end) return false;
        const auto advanced = da_handle(0x17f, cursor.handle);
        check_handle(advanced, "cannot advance SHM prefix certificate");
        require(advanced == 0 || advanced == 1, "invalid SHM prefix advance result");
        if (!advanced) return false;
        require(da_get_xvalseq(0x136, cursor.handle, &current) >= 0 &&
                da_get_xvalseq(0x146, cursor.handle, &next) >= 0, "cannot read SHM prefix bounds");
    }
    first = current.tick;
    return first <= end && da_get(0x1a3, cursor.handle) == 4 &&
           text(0x15a, cursor.handle) == "1" && next.tick > end;
}
bool final_bits(Cursor &cursor, std::uint64_t end, std::uint32_t width,
                std::uint64_t &at, std::string &value) {
    const auto sought = da_handle_pl(0x17f, cursor.handle, 0x126, SstTime{end, UINT64_MAX}, 0x10);
    check_handle(sought, "cannot seek SHM point");
    require(sought == 0 || sought == 1, "invalid SHM point seek result");
    if (!sought) return false;
    SstTime current{}, next{};
    require(da_get_xvalseq(0x136, cursor.handle, &current) >= 0 &&
            da_get_xvalseq(0x146, cursor.handle, &next) >= 0, "cannot read SHM point time");
    if (current.tick > end || next.tick <= end || da_get(0x1a3, cursor.handle) < 0) return false;
    value = text(0x15a, cursor.handle);
    if (value.size() != width || value.find_first_not_of("01xz") != std::string::npos) return false;
    at = current.tick;
    return true;
}
bool certified_point(const Declaration &d, Cursor &cursor, std::uint64_t end,
                     std::uint64_t &changed_at, std::string &value) {
    std::uint64_t first = 0;
    if (!point_prefix(d, end, first)) return false;
    std::uint64_t at = 0;
    if (!final_bits(cursor, end, d.descriptor.width, at, value)) return false;
    changed_at = 0;
    if (at < first) return false;
    while (at > first) {
        std::uint64_t previous = 0;
        std::string before;
        if (!final_bits(cursor, at - 1, d.descriptor.width, previous, before)) return false;
        require(previous < at, "SHM point predecessor did not move backwards");
        if (before != value) { changed_at = at; break; }
        at = previous;
    }
    return true;
}
struct Record {
    std::uint64_t id, tick, sequence;
    std::uint32_t encoding;
    bool failed;
    // Own either the payload bytes or the failure text. Short payloads need no
    // separate allocation, and successful records carry no empty error string.
    std::string data;
    std::uint64_t occurrences = 0;
};
bool before(const Record &a, const Record &b) {
    if (a.tick != b.tick) return a.tick < b.tick;
    if (a.id != b.id) return a.id < b.id;
    return a.sequence < b.sequence;
}
Record decode(const Declaration &declaration, Cursor &cursor, SstTime current, int state) {
    const auto &d = declaration.descriptor;
    Record record{d.id, current.tick, current.sequence, d.encoding, false, {}};
    require(state >= 0, "SHM history has an unavailable recording interval");
    if (d.encoding == 1) {
        record.data = text(0x15a, cursor.handle);
        require(record.data.size() == d.width, "SHM bit value width disagrees with declaration");
        for (char &stored : record.data) {
            if (stored == '0' || stored == '1') continue;
            unsigned char bit = static_cast<unsigned char>(stored);
            if (bit >= 'A' && bit <= 'Z') bit += 'a' - 'A';
            require(std::strchr("01xzuwlh-", bit) && bit, "invalid SHM logic state");
            stored = static_cast<char>(bit);
        }
    } else if (d.encoding == 2) {
        double real = 0;
        require(da_get_real(0x107, cursor.handle, &real) >= 0, "cannot read SHM real value");
        record.data.resize(sizeof(real));
        std::memcpy(record.data.data(), &real, sizeof(real));
    } else if (d.encoding == 3) {
        const char *encoded = da_get_sstr_pl(0x15a, cursor.handle, 0x1b3, 5, 0x204, 0x200, 0x10);
        record.data = ondas_shm_value::decode_quoted(encoded);
    } else if (d.encoding == 4) {
        require(state == 2, "unsupported SHM event state");
        record.occurrences = 1;
    } else throw std::runtime_error("unsupported SHM value encoding");
    return record;
}
bool earlier(SstTime a, SstTime b) {
    return a.tick < b.tick || (a.tick == b.tick && a.sequence < b.sequence);
}
SstTime current_time(Cursor &cursor) {
    SstTime current{};
    require(da_get_xvalseq(0x136, cursor.handle, &current) >= 0, "cannot read SHM record time");
    return current;
}
bool advance(Cursor &cursor, SstTime current, std::uint64_t end) {
    SstTime next{};
    require(da_get_xvalseq(0x146, cursor.handle, &next) >= 0, "cannot read SHM next time");
    if (next.sequence == UINT64_MAX || next.tick > end) return false;
    const auto advanced = da_handle(0x17f, cursor.handle);
    check_handle(advanced, "cannot advance SHM value iterator");
    require(advanced == 0 || advanced == 1, "invalid SHM advance result");
    if (!advanced) return false;
    require(earlier(current, current_time(cursor)), "SHM history is not in source order");
    return true;
}
struct History {
    Record head{};
    bool ready = false, done = false, available = false, checkpoint = false;
    std::uint64_t through = 0;

    template<class Account>
    void fill(const Declaration &declaration, Cursor &cursor, std::uint64_t end,
              ondas_shm_work &work, Account account) {
        while (!ready && !done) {
            auto current = current_time(cursor);
            if (current.tick > end) { done = true; break; }
            const auto tick = current.tick;
            do {
                ++work.history_records;
                const int state = da_get(0x1a3, cursor.handle);
                try {
                    if (!(declaration.descriptor.encoding == 4 && state == 1) && !(state < 0 && !available)) {
                        available = true;
                        auto value = decode(declaration, cursor, current, state);
                        account(&value);
                        // Rust preserves the typed ASCII-domain error. Do not
                        // overwrite an invalid intermediate string at this tick.
                        const bool invalid_string = value.encoding == 3 &&
                            std::any_of(value.data.begin(), value.data.end(), [](unsigned char c) { return c > 127; });
                        if (ready && value.encoding == 4) {
                            require(head.occurrences != UINT64_MAX, "SHM event count overflow");
                            value.occurrences += head.occurrences;
                        }
                        head = std::move(value);
                        ready = true;
                        if (invalid_string) { done = true; break; }
                    }
                    if (!advance(cursor, current, end)) { done = true; break; }
                    current = current_time(cursor);
                } catch (const std::exception &failure) {
                    head = {declaration.descriptor.id, tick, current.sequence,
                            declaration.descriptor.encoding, true, failure.what()};
                    ready = true; done = true;
                    account(nullptr);
                    break;
                }
            } while (current.tick == tick);
            through = tick;
            checkpoint = true;
        }
    }
};

struct Stream {
    const std::string &path;
    const std::vector<Declaration> &imported;
    const std::vector<SourceNode> &nodes;
    ondas_shm_work &work;
    std::vector<std::uint64_t> ids;
    std::uint64_t from, end;
    bool resume;
    std::vector<History> histories;
    std::unique_ptr<Session> session;
    std::vector<Declaration> declarations;
    std::vector<std::unique_ptr<Cursor>> cursors;
    std::vector<size_t> heap;
    size_t refill = root_node;
    bool started = false, activated = false, first_batch = true;
    Record output{};
    std::uint64_t queued_payload = 0;

    Stream(const std::string &path, const std::vector<Declaration> &imported,
           const std::vector<SourceNode> &nodes, ondas_shm_work &work,
           const std::uint64_t *ids, size_t count, std::uint64_t from,
           std::uint64_t end, bool resume)
        : path(path), imported(imported), nodes(nodes), work(work),
          ids(ids, ids + count), from(from), end(end), resume(resume), histories(count) {
        heap.reserve(count);
        for (auto &history : histories) history.available = resume;
    }
    void suspend() {
        cursors.clear();
        declarations.clear();
        session.reset();
    }
    void activate();
    ~Stream();
    bool later(size_t a, size_t b) const { return before(histories[b].head, histories[a].head); }
    void fill(size_t index) {
        auto &h = histories[index];
        if (!h.done) h.fill(declarations[index], *cursors[index], end, work,
                            [this](const Record *record) { account(record); });
        if (h.ready) {
            heap.push_back(index);
            std::push_heap(heap.begin(), heap.end(), [this](size_t a, size_t b) { return later(a, b); });
        }
    }
    void account(const Record *transient = nullptr) {
        std::uint64_t payload = queued_payload + output.data.capacity();
        for (const auto &h : histories) payload += h.head.data.capacity();
        if (transient) payload += transient->data.capacity();
        work.pending_records = std::max<std::uint64_t>(work.pending_records, histories.capacity() + 2);
        work.pending_payload = std::max(work.pending_payload, payload);
    }
    const Record *next() {
        if (!started || (refill != root_node && !histories[refill].done)) activate();
        if (!started) {
            for (size_t i = 0; i < histories.size(); ++i) fill(i);
            started = true;
        }
        if (refill != root_node) { fill(refill); refill = root_node; }
        if (heap.empty()) return nullptr;
        std::pop_heap(heap.begin(), heap.end(), [this](size_t a, size_t b) { return later(a, b); });
        const auto index = heap.back(); heap.pop_back();
        output = std::move(histories[index].head);
        histories[index].head = {};
        histories[index].ready = false;
        refill = index;
        account();
        return &output;
    }
};
Stream *active_stream = nullptr;
void suspend_active() {
    if (auto *stream = active_stream) {
        active_stream = nullptr;
        stream->suspend();
    }
}
Stream::~Stream() {
    if (active_stream == this) active_stream = nullptr;
    suspend();
}
void Stream::activate() {
    if (active_stream == this) return;
    // Session suspends the previous stream before touching SST2's shared state.
    try {
        if (!session) {
            session = std::make_unique<Session>(path);
            if (activated) ++work.reopens;
            else ++work.sessions;
            declarations = selected_declarations(session->handle, ids.data(), ids.size(), imported, nodes, work);
        }
        cursors.resize(ids.size());
        for (size_t i = 0; i < ids.size(); ++i) {
            auto &history = histories[i];
            if (history.done) continue;
            if (activated) ++work.reloaded_histories;
            else ++work.histories;
            cursors[i] = std::make_unique<Cursor>(declarations[i].variable);
            auto &cursor = *cursors[i];
            if (history.checkpoint || resume) {
                const auto boundary = history.checkpoint ? history.through : from - 1;
                const auto sought = da_handle_pl(0x17f, cursor.handle, 0x126, SstTime{boundary, UINT64_MAX}, 0x10);
                check_handle(sought, "cannot seek SHM completed tick");
                require(sought == 0 || sought == 1, "invalid SHM seek result");
                if (!sought) { history.done = true; continue; }
                auto current = current_time(cursor);
                require(current.tick <= boundary, "SHM seek skipped the resume boundary");
                while (current.tick <= boundary) {
                    if (history.checkpoint) ++work.replayed_records;
                    if (!advance(cursor, current, end)) { history.done = true; break; }
                    current = current_time(cursor);
                }
            }
        }
        active_stream = this;
        activated = true;
    } catch (...) { suspend(); throw; }
}

}

struct ondas_shm {
    std::string path;
    ondas_shm_meta meta{};
    std::vector<Declaration> declarations;
    std::vector<SourceNode> source_nodes;
    ondas_shm_work work{};
    std::unique_ptr<Stream> stream;
    std::vector<Record> batch;
    size_t position = 0;
    std::vector<Record> points;
    bool point_ready = false;
    std::uint64_t point_end = 0;
    std::vector<std::uint64_t> point_ids;
};
extern "C" int ondas_shm_open(const char *path, int metadata_only, ondas_shm **out, char *error, size_t cap, int *os_error) {
    *out = nullptr;
    *os_error = 0;
    try {
        auto reader = std::make_unique<ondas_shm>();
        reader->path = path;
        Session session(reader->path);
        ++reader->work.sessions;
        reader->meta = metadata(session.handle);
        if (!metadata_only) {
            std::unordered_map<SstHandle, std::uint64_t> identities;
            hierarchy(session.handle, reader->declarations, identities, reader->source_nodes);
            reader->work.hierarchy_nodes += reader->source_nodes.size();
            for (auto &d : reader->declarations) d.variable = 0;
        }
        *out = reader.release();
        return 0;
    } catch (const Unavailable &failure) { report(error, cap, failure.what()); return -3; }
    catch (const std::filesystem::filesystem_error &failure) { return filesystem_failure(failure, os_error, error, cap); }
    catch (const std::exception &failure) { report(error, cap, failure.what()); }
    catch (...) { report(error, cap, "unexpected SHM native exception"); }
    return -1;
}
extern "C" void ondas_shm_close(ondas_shm *reader) { delete reader; }
extern "C" void ondas_shm_metadata(const ondas_shm *reader, ondas_shm_meta *out) { *out = reader->meta; }
extern "C" void ondas_shm_work_done(const ondas_shm *reader, ondas_shm_work *out) { *out = reader->work; }
extern "C" size_t ondas_shm_decl_count(const ondas_shm *reader) { return reader->declarations.size(); }
extern "C" void ondas_shm_declaration(const ondas_shm *reader, size_t index, ondas_shm_decl *out) {
    const auto &d = reader->declarations[index];
    *out = d.descriptor;
    out->name = d.name.c_str(); out->kind = d.kind.c_str();
    out->definition = d.definition.empty() ? nullptr : d.definition.c_str();
    out->variant_count = d.variants.size();
    out->enum_name = d.enum_name.empty() ? nullptr : d.enum_name.c_str();
}
extern "C" void ondas_shm_enum_variant(const ondas_shm *reader, size_t declaration,
                                       size_t variant, ondas_shm_variant *out) {
    const auto &v = reader->declarations[declaration].variants[variant];
    *out = {v.first.c_str(), v.second.c_str()};
}
extern "C" int ondas_shm_read(ondas_shm *reader, const uint64_t *ids, size_t count, uint64_t from, uint64_t end, int validated_resume, char *error, size_t cap, int *os_error) {
    *os_error = 0;
    const bool prepared = reader->point_ready && reader->stream && !reader->stream->started &&
        from == 0 && !validated_resume && end == reader->point_end &&
        count == reader->point_ids.size() && std::equal(reader->point_ids.begin(), reader->point_ids.end(), ids);
    reader->point_ready = false; reader->point_ids.clear();
    reader->points.clear();
    if (prepared) return 0;
    reader->stream.reset(); reader->batch.clear(); reader->position = 0;
    try {
        require(!validated_resume || from > 0, "invalid SHM resume boundary");
        reader->stream = std::make_unique<Stream>(reader->path, reader->declarations,
            reader->source_nodes, reader->work, ids, count, from, end, validated_resume != 0);
        reader->stream->activate();
        return 0;
    } catch (const Unavailable &failure) {
        reader->stream.reset(); report(error, cap, failure.what()); return -3;
    } catch (const std::filesystem::filesystem_error &failure) {
        reader->stream.reset(); return filesystem_failure(failure, os_error, error, cap);
    } catch (const std::exception &failure) { report(error, cap, failure.what()); }
    catch (...) { report(error, cap, "unexpected SHM native exception"); }
    reader->stream.reset();
    return -1;
}

extern "C" int ondas_shm_sample_bits(ondas_shm *reader, const uint64_t *ids, size_t count,
                                      uint64_t end, int prepare_fallback, ondas_shm_point *out) {
    reader->point_ready = false; reader->point_ids.clear();
    reader->stream.reset(); reader->points.clear(); reader->batch.clear(); reader->position = 0;
    if (!count || end == UINT64_MAX || end > reader->meta.last) return 0;
    try {
        auto session = std::make_unique<Session>(reader->path);
        ++reader->work.sessions;
        const auto declarations = selected_declarations(session->handle, ids, count,
            reader->declarations, reader->source_nodes, reader->work);
        reader->points.resize(count);
        std::vector<size_t> fallback;
        for (size_t i = 0; i < count; ++i) {
            out[i] = {};
            auto &point = reader->points[i];
            const auto &declaration = declarations[i];
            if (point_type(declaration)) {
                // Release each cursor before loading another history. These
                // operations retain only owned point bytes, not SDK histories.
                Cursor cursor(declaration.variable);
                if (certified_point(declaration, cursor, end, point.tick, point.data)) {
                    out[i] = {point.tick, reinterpret_cast<const std::uint8_t *>(point.data.data()),
                              point.data.size(), point.tick != 0, 1};
                    continue;
                }
            }
            point.data.clear();
            fallback.push_back(i);
        }
        if (fallback.empty()) return 1;
        const bool partial = fallback.size() != count;
        if (!partial && !prepare_fallback) { reader->points.clear(); return 0; }
        // Prepare only identities. No fallback history is collected before delivery.
        for (size_t index : fallback) reader->point_ids.push_back(ids[index]);
        reader->stream = std::make_unique<Stream>(reader->path, reader->declarations,
            reader->source_nodes, reader->work, reader->point_ids.data(), fallback.size(), 0, end, false);
        reader->stream->session = std::move(session);
        for (size_t index : fallback) reader->stream->declarations.push_back(declarations[index]);
        reader->stream->activate();
        reader->point_end = end;
        reader->point_ready = true;
        return partial ? 2 : 0;
    } catch (...) {
        // Structural/SDK exceptions leave no reusable partial batch. Timed
        // payload failures in collected ordinary records remain ordered data.
        reader->point_ready = false; reader->point_ids.clear();
        reader->stream.reset(); reader->points.clear();
        return 0;
    }
}
extern "C" int ondas_shm_next(ondas_shm *reader, ondas_shm_record *out, char *error, size_t cap, int *os_error) {
    *os_error = 0;
    try {
        if (!reader->stream) return 0;
        if (reader->position == reader->batch.size()) {
            reader->batch.clear(); reader->position = 0;
            reader->stream->queued_payload = 0;
            // Amortize session restoration between interleaved readers. Both
            // row count and payload are bounded; one value can cross the byte cap.
            constexpr size_t batch_payload = 64 * 1024;
            const size_t batch_records = reader->stream->first_batch ? 64 : 1024;
            reader->stream->first_batch = false;
            reader->batch.reserve(batch_records);
            while (reader->batch.size() < batch_records) {
                const auto *next = reader->stream->next();
                if (!next) break;
                const bool failed = next->failed;
                reader->batch.push_back(std::move(reader->stream->output));
                reader->stream->output = {};
                reader->stream->queued_payload += reader->batch.back().data.capacity();
                reader->stream->account();
                if (failed || reader->stream->queued_payload >= batch_payload) break;
            }
            reader->work.pending_records = std::max<std::uint64_t>(reader->work.pending_records,
                reader->stream->histories.capacity() + reader->batch.capacity() + 2);
        }
        if (reader->position == reader->batch.size()) return 0;
        const auto *record = &reader->batch[reader->position++];
        if (!reader->work.delivered) reader->work.first_delivery_records = reader->work.history_records;
        ++reader->work.delivered;
        *out = {record->id, record->tick, record->encoding, nullptr, 0, record->occurrences};
        if (record->failed) {
            if (cap) std::snprintf(error, cap, "history %llu at tick %llu: %s",
                                  static_cast<unsigned long long>(record->id),
                                  static_cast<unsigned long long>(record->tick), record->data.c_str());
            return -1;
        }
        out->data = reinterpret_cast<const std::uint8_t *>(record->data.data());
        out->len = record->data.size();
        return 1;
    } catch (const std::filesystem::filesystem_error &failure) {
        const auto code = filesystem_failure(failure, os_error, error, cap);
        return code == -2 ? -2 : -4;
    } catch (const Unavailable &failure) { report(error, cap, failure.what()); return -3; }
    catch (const std::exception &failure) { report(error, cap, failure.what()); }
    catch (...) { report(error, cap, "unexpected SHM native exception"); }
    return -4;
}
extern "C" void ondas_shm_end(ondas_shm *reader) {
    reader->point_ready = false; reader->point_ids.clear();
    reader->stream.reset();
    std::vector<Record>().swap(reader->batch);
    std::vector<Record>().swap(reader->points);
    reader->position = 0;
}
