#include "vwdb.h"
#include "vwdb_abi.h"
#include <algorithm>
#include <cstdio>
#include <cstring>
#include <dlfcn.h>
#include <filesystem>
#include <limits>
#include <memory>
#include <stdexcept>
#include <string>
#include <tuple>
#include <unordered_map>
#include <unordered_set>
#include <utility>
#include <vector>
#include <zlib.h>

namespace {
using namespace vwdb;
constexpr size_t root = std::numeric_limits<size_t>::max();
const Native success = reinterpret_cast<Native>(1);
constexpr int missing = 0, bits = 1, real = 2, event = 4;

class Unavailable : public std::runtime_error {
    using std::runtime_error::runtime_error;
};

class Unsupported : public std::runtime_error {
    using std::runtime_error::runtime_error;
};

void require(bool condition, const char *message) {
    if (!condition) throw std::runtime_error(message);
}

template<class F>
F load(void *library, const char *symbol) {
    auto fn = reinterpret_cast<F>(dlsym(library, symbol));
    if (!fn) {
        const char *detail = dlerror();
        throw Unavailable(std::string("VWDB reader ABI is missing ") + symbol +
                          (detail ? std::string(": ") + detail : ""));
    }
    return fn;
}

const Abi &abi(const char *path = nullptr, const char **resolved_path = nullptr) {
    // The reader needs zlib symbols but does not list zlib in DT_NEEDED.
    // Reference zlib here so downstream --as-needed keeps the dependency.
    static const auto loaded = [path] {
        if (!zlibVersion()) throw Unavailable("cannot initialize zlib");
        std::error_code error;
        auto canonical = std::filesystem::canonical(path, error);
        if (error) throw Unavailable("cannot resolve VWDB reader: " + error.message());
        void *library = dlopen(canonical.c_str(), RTLD_NOW | RTLD_LOCAL | RTLD_DEEPBIND);
        if (!library) throw Unavailable(dlerror());
        try {
#define BIND(field, symbol) load<decltype(Abi::field)>(library, symbol)
            Abi a{
                BIND(init, "_Z15vwdbrAnalogInitv"),
                BIND(open, "_ZN11vwdbrAnalog10vrOpenFileEPKcPP10datHandleAPFiPviES5_"),
                BIND(close, "_ZN11vwdbrAnalog7vrCloseEP10datHandleA"),
                BIND(precision, "_ZN11vwdbrAnalog21vrGlobalTimePrecisionEPj"),
                BIND(first, "_ZN11vwdbrAnalog16vrScopeStartTimeEP10datHandleAP10SimTimeSeq"),
                BIND(last, "_ZN11vwdbrAnalog14vrScopeEndTimeEP10datHandleAP10SimTimeSeq"),
                BIND(get, "da_get"), BIND(get_int, "da_get_int"),
                BIND(get_bytes, "da_get_dstr"), BIND(get_text, "da_get_sstr"),
                BIND(format, "da_get_sstr_pl"), BIND(get_stamp, "da_get_xvalseq"),
                BIND(handle, "da_handle"), BIND(configure, "da_handle_pl"),
                BIND(iterate, "da_iterate"), BIND(scan, "da_scan"), BIND(drop, "da_delete"),
                BIND(string_init, "Tcl_DStringInit"), BIND(string_free, "Tcl_DStringFree"),
                BIND(process_event, "Tcl_DoOneEvent"),
            };
#undef BIND
            a.init();
            // Initialization establishes process-lifetime SDK state.
            return std::pair{a, std::move(canonical)};
        } catch (...) {
            dlclose(library);
            throw;
        }
    }();
    if (resolved_path) *resolved_path = loaded.second.c_str();
    return loaded.first;
}

struct Delete {
    void operator()(Native handle) const {
        if (handle != success) abi().drop(handle);
    }
};
using Handle = std::unique_ptr<Object, Delete>;

Handle checked(Native native) {
    Handle handle(native);
    if (handle && handle.get() != success && abi().get(Error, handle.get()) > 0) {
        const char *message = abi().get_text(ErrorMessage, handle.get());
        throw std::runtime_error(message ? message : "unknown native VWDB error");
    }
    return handle;
}

class Iterator {
    Handle cursor;
public:
    Iterator(int relation, Native owner) : cursor(checked(abi().iterate(relation, owner))) {
        require(cursor.get() != success, "invalid VWDB iterator");
    }
    Handle next() {
        if (!cursor) return {};
        auto child = abi().scan(cursor.get());
        // The SDK destroys an exhausted iterator as part of the null scan.
        if (!child) cursor.release();
        require(child != success, "invalid VWDB child");
        return checked(child);
    }
};

void drain_ready_events() {
    // The SDK defers handle reclamation to its Tcl scheduler.
    while (abi().process_event(tcl_dont_wait | tcl_timer_events | tcl_idle_events)) {}
}

void close_database(Native database) {
    abi().close(database);
    drain_ready_events();
}

class Session {
    Native database = nullptr;
public:
    explicit Session(const std::string &path) {
        const int status = abi().open(path.c_str(), &database, nullptr, nullptr);
        if (status != 0 || !database) {
            if (database) close_database(database);
            throw std::runtime_error("cannot open VWDB (SDK status " + std::to_string(status) + ")");
        }
    }
    Session(const Session &) = delete;
    Session &operator=(const Session &) = delete;
    ~Session() { close_database(database); }
    Native get() const { return database; }
};

ondas_vwdb_meta metadata(Native database) {
    unsigned precision = 0;
    require(abi().precision(&precision) == 0 && precision >= 21 && precision <= 36,
            "unsupported VWDB time precision");
    Stamp first{}, last{};
    require(abi().first(database, &first) == 0 && abi().last(database, &last) == 0 &&
            first.tick <= last.tick, "cannot read VWDB recording interval");
    return {first.tick, last.tick, static_cast<int32_t>(precision) - 36};
}

std::string name(Native object) {
    const char *text = abi().get_text(Name, object);
    require(text, "missing native VWDB name");
    return text;
}

int extent(int property, Native object) {
    int result = -1;
    return abi().get_int(property, object, &result) < 0 ? -1 : result;
}

int type_kind(Native type) { return type ? abi().get(TypeKind, type) : -1; }

Handle object_type(Native object) {
    auto type = checked(abi().handle(Type, object));
    require(type.get() != success, "invalid native VWDB type");
    return type;
}

struct Declaration {
    ondas_vwdb_decl descriptor{};
    int relation, type_kind = -1, raw_bits = -1;
    bool raw_enum = false;
    size_t ordinal;
    std::string name;
    std::vector<std::pair<std::string, std::string>> variants;
};

std::string binary_bytes(const char *data, size_t size) {
    std::string bits;
    bits.reserve(size * 8);
    for (size_t i = 0; i < size; ++i) {
        const auto byte = static_cast<unsigned char>(data[i]);
        for (int bit = 7; bit >= 0; --bit) bits.push_back((byte >> bit) & 1 ? '1' : '0');
    }
    return bits;
}

void import_range(Declaration &decl, Native type) {
    auto &d = decl.descriptor;
    if (decl.type_kind != Vector || abi().get(PackedLayout, type) != 0 ||
        abi().get(HasRange, type) <= 0) return;
    auto index = checked(abi().handle(IndexType, type));
    if (!index || index.get() == success) return;
    const int width = extent(RawBits, index.get());
    const int count = extent(ElementCount, type);
    if ((width != 32 && width != 64) || count <= 0 || static_cast<uint32_t>(count) != d.width) return;
    const char *bytes = abi().get_text(FirstIndex, type);
    require(bytes, "missing VWDB packed range bound");
    uint64_t raw = 0;
    for (int i = 0; i < width / 8; ++i) raw = (raw << 8) | static_cast<unsigned char>(bytes[i]);
    d.msb = width == 32 ? static_cast<int32_t>(raw) : static_cast<int64_t>(raw);
    const int64_t offset = abi().get(Ascending, type) > 0 ? count - 1LL : 1LL - count;
    require(offset >= 0 ? d.msb <= INT64_MAX - offset : d.msb >= INT64_MIN - offset,
            "VWDB packed range overflow");
    d.lsb = d.msb + offset;
    d.has_range = 1;
}

void import_enum(Declaration &decl, Native type) {
    Iterator iterator(EnumElements, type);
    while (auto label = iterator.next()) {
        auto label_name = name(label.get());
        require(decl.raw_bits > 0, "invalid VWDB enum storage width");
        const char *raw = abi().get_text(RawValue, label.get());
        require(raw, "missing VWDB enum key");
        // The formatter and accessors share temporary SDK buffers.
        const std::string copy(raw, (static_cast<size_t>(decl.raw_bits) + 7) / 8);
        if (decl.raw_enum) {
            decl.variants.emplace_back(binary_bytes(copy.data(), copy.size()), std::move(label_name));
            continue;
        }
        const char *encoded = abi().format(FormattedValue, type, RawValue, copy.data(),
            static_cast<int>(copy.size()), ValueFormat, binary_format, EndOptions);
        require(encoded, "cannot format VWDB enum key");
        decl.variants.emplace_back(encoded, std::move(label_name));
    }
}

void inventory(Native owner, std::vector<Declaration> &decls, size_t parent = root,
               size_t depth = 0) {
    if (depth >= 256) throw Unsupported("VWDB hierarchy exceeds 256 levels");
    for (int relation : {Scopes, Variables}) {
        Iterator children(relation, owner);
        for (size_t ordinal = 0; auto child = children.next(); ++ordinal) {
            auto type = object_type(child.get());
            Declaration decl{};
            decl.relation = relation;
            decl.ordinal = ordinal;
            decl.name = name(child.get());
            auto &d = decl.descriptor;
            d.parent = parent;
            decl.type_kind = type_kind(type.get());
            const bool scope = relation == Scopes || decl.type_kind == Array || decl.type_kind == Record;
            d.native_kind = scope ? abi().get(ScopeKind, child.get()) : decl.type_kind;
            d.encoding = scope ? -1 : 0;
            if (scope) {
                d.indexed_children = d.native_kind == ArrayScope || decl.type_kind == Array;
                if (d.indexed_children) d.kind = "array";
                else if (decl.type_kind == Record) d.kind = "record";
                else if (d.native_kind == ArchitectureScope) d.kind = "architecture";
                else if (d.native_kind == ModuleScope) d.kind = "module";
            } else if (type) {
                decl.raw_bits = extent(RawBits, type.get());
                const int width = extent(Width, type.get());
                const int kind = decl.type_kind;
                // Enums without a logical width store byte-aligned codes
                // directly. Their formatter returns labels instead of bits.
                decl.raw_enum = kind == Enum && width <= 0 && decl.raw_bits > 0 && decl.raw_bits % 8 == 0;
                if (decl.raw_enum ||
                    ((kind == Scalar || kind == Enum || kind == Vector || kind == Character) && width > 0)) {
                    d.encoding = bits;
                    d.width = static_cast<uint32_t>(decl.raw_enum ? decl.raw_bits : width);
                    import_range(decl, type.get());
                } else if (kind == Real && (decl.raw_bits == 32 || decl.raw_bits == 64)) {
                    d.encoding = real;
                } else if (kind == Event) {
                    d.encoding = event;
                }
                d.is_constant = abi().get(IsConstant, child.get()) > 0;
                import_enum(decl, type.get());
            }
            const size_t index = decls.size();
            decls.push_back(std::move(decl));
            if (scope) inventory(child.get(), decls, index, depth + 1);
        }
    }
}

using Routes = std::unordered_map<size_t, std::vector<size_t>>;
using Resolved = std::unordered_map<size_t, Handle>;
void resolve(Native owner, size_t parent, const std::vector<Declaration> &decls,
             Routes &routes, Resolved &out) {
    const auto entry = routes.find(parent);
    if (entry == routes.end()) return;
    auto &wanted = entry->second;
    std::sort(wanted.begin(), wanted.end(), [&](size_t a, size_t b) {
        return std::tie(decls[a].relation, decls[a].ordinal) <
               std::tie(decls[b].relation, decls[b].ordinal);
    });
    for (size_t i = 0; i < wanted.size();) {
        const int relation = decls[wanted[i]].relation;
        Iterator iterator(relation, owner);
        size_t ordinal = 0;
        do {
            const auto id = wanted[i++];
            const auto &decl = decls[id];
            Handle object;
            do {
                object = iterator.next();
                require(bool(object), "VWDB hierarchy changed after open");
            } while (ordinal++ < decl.ordinal);
            require(name(object.get()) == decl.name, "VWDB hierarchy changed after open");
            const auto child = object.get();
            out.emplace(id, std::move(object));
            resolve(child, id, decls, routes, out);
        } while (i < wanted.size() && decls[wanted[i]].relation == relation);
    }
}

class Cursor {
    Native signal;
    Handle cursor;
    bool loaded = false;
public:
    explicit Cursor(Native signal) : signal(signal) {}
    Cursor(const Cursor &) = delete;
    Cursor &operator=(const Cursor &) = delete;
    ~Cursor() {
        cursor.reset();
        if (loaded) {
            Handle response(abi().handle(Unload, signal));
        }
    }
    void start() {
        require(checked(abi().handle(Load, signal)).get() == success, "cannot load VWDB history");
        loaded = true;
        cursor = checked(abi().configure(History, signal, AllowSummary, 0,
                                         ValueFormat, binary_format, EndOptions));
        require(cursor.get() != success, "invalid VWDB history cursor");
    }
    Native get() const { return cursor.get(); }
    Stamp time() const {
        Stamp time{};
        require(abi().get_stamp(Time, get(), &time) >= 0, "cannot read VWDB history time");
        return time;
    }
    bool advance(Stamp previous) {
        const auto next = checked(abi().handle(NextValue, get()));
        require(!next || next.get() == success, "invalid VWDB history advance");
        if (!next) return false;
        const auto current = time();
        require(std::tie(previous.tick, previous.sequence) <= std::tie(current.tick, current.sequence),
                "VWDB history moves backwards");
        return true;
    }
};

struct Payload {
    int encoding;
    std::string bytes;
};

Payload read_value(const Declaration &decl, Native cursor) {
    const int state = abi().get(State, cursor);
    const int encoding = decl.descriptor.encoding;
    if (encoding == event) {
        require(state == 2, "invalid VWDB event state");
        return {event, {}};
    }
    if (encoding == real || decl.raw_enum) {
        if (state == -1) return {missing, {}};
        DString raw{};
        abi().string_init(&raw);
        std::unique_ptr<DString, decltype(Abi::string_free)> storage(&raw, abi().string_free);
        require(abi().get_bytes(RawValue, cursor, &raw) >= 0 && raw.length == decl.raw_bits / 8,
                "cannot read VWDB raw value");
        if (decl.raw_enum) return {bits, binary_bytes(raw.string, static_cast<size_t>(raw.length))};
        return {real, std::string(raw.string, raw.length)};
    }
    require(encoding == bits, "unsupported VWDB encoding");
    const char *value = abi().format(FormattedValue, cursor, FormatOptions, full_width, EndOptions);
    if (!value && state == -1) return {missing, {}};
    require(value && std::strlen(value) == decl.descriptor.width, "invalid VWDB bit width");
    return {bits, value};
}

struct HistoryState {
    size_t id;
    size_t consumed = 0;
    Stamp head{};
    bool finished = false;
};

struct Stream;
struct SessionSlot {
    std::unique_ptr<Session> session;
    Stream *stream = nullptr;

    ~SessionSlot() { close(); }
    void open(const std::string &path);
    void close();
};

// Serialized by the Rust SDK lock. Only one live reader holds a native session.
SessionSlot *active = nullptr;

struct Stream {
    SessionSlot &owner;
    const std::string &path;
    const std::vector<Declaration> &decls;
    uint64_t end;
    std::vector<HistoryState> histories;
    Resolved resolved;
    std::vector<std::unique_ptr<Cursor>> cursors;
    std::vector<size_t> owned_ids;
    std::vector<size_t> heap;
    std::string output;
    size_t refill = root;

    Stream(SessionSlot &owner, const std::string &path, const std::vector<Declaration> &decls,
           const size_t *ids, size_t count, uint64_t end)
        : owner(owner), path(path), decls(decls), end(end) {
        histories.reserve(count);
        for (size_t i = 0; i < count; ++i) {
            require(ids[i] < decls.size() && decls[ids[i]].descriptor.encoding > 0,
                    "invalid VWDB history identity");
            histories.push_back({ids[i]});
        }
    }
    ~Stream() { suspend(); }

    void suspend() {
        // Cursors unload through live signal handles. Drop children before
        // closing their common database.
        cursors.clear();
        // The declaration import is preorder. Release children before parents.
        for (size_t id : owned_ids) resolved.erase(id);
        owned_ids.clear();
        resolved.clear();
        heap.clear();
        refill = root;
        if (owner.stream == this) owner.stream = nullptr;
    }

    bool later(size_t lhs, size_t rhs) const {
        const auto &a = histories[lhs];
        const auto &b = histories[rhs];
        return std::tie(a.head.tick, a.id) > std::tie(b.head.tick, b.id);
    }
    auto compare() { return [this](size_t a, size_t b) { return later(a, b); }; }

    void queue(size_t index) {
        auto &history = histories[index];
        auto &cursor = *cursors[index];
        while (!history.finished) {
            history.head = cursor.time();
            if (history.head.tick > end) {
                history.finished = true;
            } else if (decls[history.id].descriptor.encoding == event &&
                       abi().get(State, cursor.get()) != 2) {
                const int state = abi().get(State, cursor.get());
                if (state != 1 && state != -1) break;
                ++history.consumed;
                history.finished = !cursor.advance(history.head);
            } else break;
        }
        if (!history.finished) {
            heap.push_back(index);
            std::push_heap(heap.begin(), heap.end(), compare());
        } else cursors[index].reset();
    }

    void activate() {
        if (owner.stream == this) return;
        try {
            owner.open(path);
            Routes routes;
            // One route per shared ancestor, using only owned native ordinals.
            std::unordered_set<size_t> seen;
            for (const auto &history : histories) {
                if (history.finished) continue;
                for (size_t id = history.id; id != root && seen.insert(id).second;
                     id = decls[id].descriptor.parent) {
                    routes[decls[id].descriptor.parent].push_back(id);
                    owned_ids.push_back(id);
                }
            }
            std::sort(owned_ids.rbegin(), owned_ids.rend());
            resolve(owner.session->get(), root, decls, routes, resolved);
            cursors.resize(histories.size());
            for (size_t i = 0; i < histories.size(); ++i) {
                auto &history = histories[i];
                if (history.finished) continue;
                const auto &decl = decls[history.id];
                auto signal = resolved.at(history.id).get();
                auto type = object_type(signal);
                require(type_kind(type.get()) == decl.type_kind && type &&
                        extent(RawBits, type.get()) == decl.raw_bits &&
                        (decl.descriptor.encoding != bits || (decl.raw_enum
                            ? extent(Width, type.get()) <= 0
                            : extent(Width, type.get()) == static_cast<int>(decl.descriptor.width))),
                        "VWDB declaration changed after open");
                auto &cursor = cursors[i];
                cursor = std::make_unique<Cursor>(signal);
                cursor->start();
                // Equal tick/sequence pairs can occur, so resume by source ordinal.
                bool more = cursor->get() != nullptr;
                for (size_t skipped = 0; skipped < history.consumed; ++skipped) {
                    require(more, "VWDB history changed after open");
                    more = cursor->advance(cursor->time());
                }
                history.finished = !more;
                if (more) queue(i);
            }
            owner.stream = this;
        } catch (...) {
            suspend();
            owner.close();
            throw;
        }
    }

    bool next(ondas_vwdb_record &record) {
        activate();
        if (refill != root) {
            const size_t index = std::exchange(refill, root);
            auto &history = histories[index];
            history.finished = !cursors[index]->advance(history.head);
            if (!history.finished) queue(index);
            else cursors[index].reset();
        }
        if (heap.empty()) return false;
        std::pop_heap(heap.begin(), heap.end(), compare());
        const size_t index = heap.back();
        heap.pop_back();
        auto &history = histories[index];
        record = {history.id, history.head.tick, missing, nullptr, 0};
        auto value = read_value(decls[history.id], cursors[index]->get());
        output = std::move(value.bytes);
        record.encoding = value.encoding;
        record.data = reinterpret_cast<const uint8_t *>(output.data());
        record.len = output.size();
        ++history.consumed;
        refill = index;
        return true;
    }
};

void SessionSlot::open(const std::string &path) {
    if (session) return;
    if (active) active->close();
    session = std::make_unique<Session>(path);
    active = this;
}

void SessionSlot::close() {
    if (stream) stream->suspend();
    session.reset();
    if (active == this) active = nullptr;
}

template<class Operation>
int report(Operation operation, char *error, size_t cap) {
    try { return operation(); }
    catch (const Unavailable &failure) {
        if (cap) std::snprintf(error, cap, "%s", failure.what());
        return -3;
    } catch (const Unsupported &failure) {
        if (cap) std::snprintf(error, cap, "%s", failure.what());
        return -2;
    } catch (const std::exception &failure) {
        if (cap) std::snprintf(error, cap, "%s", failure.what());
    } catch (...) {
        if (cap) std::snprintf(error, cap, "unexpected native VWDB exception");
    }
    return -1;
}
}

struct ondas_vwdb {
    std::string path;
    ondas_vwdb_meta meta{};
    std::vector<Declaration> decls;
    SessionSlot owner;
    std::unique_ptr<Stream> stream;
};

extern "C" int ondas_vwdb_ready(const char *library, const char **resolved_path,
                                char *error, size_t cap) {
    *resolved_path = nullptr;
    return report([&] {
        abi(library, resolved_path);
        return 0;
    }, error, cap);
}

extern "C" int ondas_vwdb_open(const char *path, const char *library, int metadata_only,
                               ondas_vwdb **out, char *error, size_t cap) {
    *out = nullptr;
    return report([&] {
        abi(library);
        auto reader = std::make_unique<ondas_vwdb>();
        reader->path = path;
        reader->owner.open(reader->path);
        const auto database = reader->owner.session->get();
        reader->meta = metadata(database);
        if (metadata_only) {
            reader->owner.close();
        } else {
            inventory(database, reader->decls);
            drain_ready_events();
        }
        *out = reader.release();
        return 0;
    }, error, cap);
}
extern "C" void ondas_vwdb_close(ondas_vwdb *reader) { delete reader; }
extern "C" void ondas_vwdb_metadata(const ondas_vwdb *reader, ondas_vwdb_meta *out) {
    *out = reader->meta;
}
extern "C" size_t ondas_vwdb_decl_count(const ondas_vwdb *reader) { return reader->decls.size(); }
extern "C" void ondas_vwdb_declaration(const ondas_vwdb *reader, size_t index, ondas_vwdb_decl *out) {
    const auto &decl = reader->decls[index];
    *out = decl.descriptor;
    out->name = decl.name.c_str();
    out->variant_count = decl.variants.size();
}
extern "C" void ondas_vwdb_enum_variant(const ondas_vwdb *reader, size_t declaration, size_t variant,
                                       ondas_vwdb_variant *out) {
    const auto &item = reader->decls[declaration].variants[variant];
    *out = {item.first.c_str(), item.second.c_str()};
}
extern "C" int ondas_vwdb_read(ondas_vwdb *reader, const size_t *ids, size_t count, uint64_t end,
                               char *error, size_t cap) {
    reader->stream.reset();
    return report([&] {
        reader->stream = std::make_unique<Stream>(reader->owner, reader->path, reader->decls,
                                                  ids, count, std::min(end, reader->meta.last));
        return 0;
    }, error, cap);
}
extern "C" int ondas_vwdb_next(ondas_vwdb *reader, ondas_vwdb_record *out, char *error, size_t cap) {
    *out = {root, 0, 0, nullptr, 0};
    return report([&] { return reader->stream->next(*out) ? 1 : 0; }, error, cap);
}
extern "C" void ondas_vwdb_end(ondas_vwdb *reader, int keep_session) {
    reader->stream.reset();
    if (!keep_session) reader->owner.close();
    else if (reader->owner.session) drain_ready_events();
}
