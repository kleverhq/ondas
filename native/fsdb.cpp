#include "fsdb.h"
#include "ffrAPI.h"
#include <cstdio>
#include <cstring>
#include <exception>
#include <memory>
#include <stdexcept>
#include <string>
#include <unordered_map>
#include <vector>

extern "C" const void *const ondas_fsdb_dependencies[2];

namespace {
void require(bool ok, const char *message) {
    if (!ok) throw std::runtime_error(message);
}
void success(fsdbRC rc, const char *operation) {
    require(rc == FSDB_RC_SUCCESS, operation);
}
void report(char *error, size_t capacity, const char *message) noexcept {
    if (capacity) std::snprintf(error, capacity, "%s", message);
}
std::string text(const char *value) { return value ? value : ""; }
uint64_t ticks(fsdbTag64 tag) { return (uint64_t(tag.H) << 32) | tag.L; }

std::string scope_kind(unsigned type) {
    switch (type) {
    case FSDB_ST_VCD_MODULE: case FSDB_ST_SV_MODULE: return "module";
    case FSDB_ST_VCD_TASK: return "task";
    case FSDB_ST_VCD_FUNCTION: case FSDB_ST_VHDL_FUNCTION: return "function";
    case FSDB_ST_VCD_BEGIN: return "begin";
    case FSDB_ST_VCD_FORK: return "fork";
    case FSDB_ST_VCD_GENERATE: case FSDB_ST_VHDL_GENERATE: return "generate";
    case FSDB_ST_VHDL_ARCHITECTURE: return "architecture";
    case FSDB_ST_VHDL_PROCEDURE: return "procedure";
    case FSDB_ST_VHDL_RECORD: return "record";
    case FSDB_ST_VHDL_PROCESS: return "process";
    case FSDB_ST_VHDL_BLOCK: return "block";
    case FSDB_ST_VHDL_FOR_GENERATE: return "for-generate";
    case FSDB_ST_VHDL_IF_GENERATE: return "if-generate";
    case FSDB_ST_SV_INTERFACE: return "interface";
    case FSDB_ST_SV_MODPORT: return "modport";
    case FSDB_ST_SV_INTERFACEPORT_REF: return "interface-port-ref";
    case FSDB_ST_SV_MODPORT_REF: return "modport-ref";
    case FSDB_ST_SV_PACKAGE: return "package";
    case FSDB_ST_SV_PROGRAM: return "program";
    case FSDB_ST_SV_CLASS: return "class";
    default: return "fsdb:" + std::to_string(type);
    }
}
std::string var_kind(unsigned type) {
    switch (type) {
    case FSDB_VT_VCD_EVENT: return "event";
    case FSDB_VT_EVENT_VARIABLE: return "event-variable";
    case FSDB_VT_VCD_INTEGER: return "integer";
    case FSDB_VT_VCD_PARAMETER: return "parameter";
    case FSDB_VT_VCD_REAL: return "real";
    case FSDB_VT_VCD_REG: return "reg";
    case FSDB_VT_VCD_SUPPLY0: return "supply0";
    case FSDB_VT_VCD_SUPPLY1: return "supply1";
    case FSDB_VT_VCD_TIME: return "time";
    case FSDB_VT_VCD_TRI: return "tri";
    case FSDB_VT_VCD_TRIAND: return "triand";
    case FSDB_VT_VCD_TRIOR: return "trior";
    case FSDB_VT_VCD_TRIREG: return "trireg";
    case FSDB_VT_VCD_TRI0: return "tri0";
    case FSDB_VT_VCD_TRI1: return "tri1";
    case FSDB_VT_VCD_WAND: return "wand";
    case FSDB_VT_VCD_WIRE: return "wire";
    case FSDB_VT_VCD_WOR: return "wor";
    case FSDB_VT_VCD_MEMORY: return "memory";
    case FSDB_VT_VCD_MEMORY_DEPTH: return "memory-depth";
    case FSDB_VT_VCD_PORT: return "port";
    case FSDB_VT_VHDL_SIGNAL: return "vhdl-signal";
    case FSDB_VT_VHDL_VARIABLE: return "vhdl-variable";
    case FSDB_VT_SV_VARIABLE: return "variable";
    case FSDB_VT_VHDL_CONSTANT: return "vhdl-constant";
    case FSDB_VT_STRING: return "string";
    default: return "fsdb:" + std::to_string(type);
    }
}
struct Datatype {
    std::string name;
    unsigned logic = 0; // 0: not a per-bit logic representation; 1: VCD; 2: VHDL.
    std::vector<std::pair<std::string, std::string>> variants;
};
Datatype enumeration(const char *name, unsigned value_type, unsigned width,
                     unsigned count, char **labels, byte_T **values) {
    Datatype type;
    type.name = text(name);
    if (value_type == FSDB_ENUM_VALUE_TYPE_LOGIC || value_type == FSDB_ENUM_VALUE_TYPE_VERILOG_LOGIC)
        type.logic = 1;
    else if (value_type == FSDB_ENUM_VALUE_TYPE_VHDL_LOGIC) type.logic = 2;
    if (value_type >= FSDB_ENUM_VALUE_TYPE_NONE) return type;
    require(!count || (labels && values), "missing FSDB enum table");
    for (unsigned i = 0; i < count; ++i) {
        require(labels[i] && values[i] && width, "invalid FSDB enum entry");
        std::string bits(width, '0');
        if (type.logic) {
            const char *alphabet = type.logic == 2 ? "ux01zwlh-" : "01xz";
            const unsigned states = type.logic == 2 ? 9 : 4;
            for (unsigned bit = 0; bit < width; ++bit) {
                require(values[i][bit] < states, "invalid FSDB enum logic state");
                bits[bit] = alphabet[values[i][bit]];
            }
        } else {
            // Integral enum tables use native-endian scalar storage on this
            // Linux x86_64 SDK target, not one byte per logic bit.
            require(width <= 64, "unsupported FSDB integral enum width");
            uint64_t value = 0;
            std::memcpy(&value, values[i], (width + 7) / 8);
            for (unsigned bit = 0; bit < width; ++bit)
                bits[width - 1 - bit] = ((value >> bit) & 1) ? '1' : '0';
        }
        type.variants.emplace_back(std::move(bits), labels[i]);
    }
    return type;
}
struct Declaration {
    ondas_fsdb_decl data{};
    std::string name, kind, definition;
    // Only used inside C++: SDK data type and storage representation.
    unsigned dt = 0, bytes_per_bit = 0, enum_logic = 0;
};
void encoding(Declaration &d, const fsdbTreeCBDataVar &v) {
    auto &out = d.data;
    const auto width = uint64_t(std::abs(int64_t(v.lbitnum) - v.rbitnum)) + 1;
    d.dt = v.dtidcode;
    d.bytes_per_bit = v.bytes_per_bit;
    out.encoding = OFS_UNSUPPORTED;
    if (v.type == FSDB_VT_EVENT_VARIABLE) return; // Transaction data is not an HDL event.
    if (v.type == FSDB_VT_VCD_EVENT) {
        out.encoding = OFS_EVENT;
    } else if (v.type == FSDB_VT_STRING || d.dt == FSDB_DT_SV_STRING) {
        out.encoding = OFS_STRING;
    } else if (v.type == FSDB_VT_VCD_REAL || d.dt == FSDB_DT_SV_REAL ||
               d.dt == FSDB_DT_SV_SHORT_REAL || d.dt == FSDB_DT_VHDL_REAL) {
        if (width == 1 && (v.bytes_per_bit == FSDB_BYTES_PER_BIT_4B ||
                           v.bytes_per_bit == FSDB_BYTES_PER_BIT_8B)) out.encoding = OFS_REAL;
    } else if (v.bytes_per_bit == FSDB_BYTES_PER_BIT_1B &&
               (d.dt == FSDB_DT_VERILOG_STANDARD ||
                (d.dt >= FSDB_DT_VHDL_BOOLEAN && d.dt <= FSDB_DT_VHDL_SIGNED) ||
                (d.dt >= FSDB_DT_SV_LOGIC && d.dt <= FSDB_DT_SV_BYTE_UINT) ||
                d.dt == FSDB_DT_SV_TIME)) {
        require(width <= UINT32_MAX, "bit width exceeds u32");
        out.encoding = OFS_BITS;
        out.width = uint32_t(width);
        out.has_range = width > 1 || v.lbitnum != 0;
    }
}
void decode_bits(const Declaration &d, const byte_T *raw, size_t size,
                 unsigned lsb, unsigned width, uint8_t *out) {
    require(size == d.data.width && raw && out, "FSDB vector width mismatch");
    require(width && uint64_t(lsb) + width <= size, "invalid FSDB bit projection");
    const char *alphabet = "01xz";
    size_t states = 4;
    if (d.dt == FSDB_DT_VHDL_BOOLEAN || d.dt == FSDB_DT_VHDL_BIT || d.dt == FSDB_DT_VHDL_BIT_VECTOR) {
        alphabet = "01"; states = 2;
    } else if (d.enum_logic == 2 || (d.dt >= FSDB_DT_VHDL_STD_ULOGIC && d.dt <= FSDB_DT_VHDL_SIGNED)) {
        alphabet = "ux01zwlh-"; states = 9;
    }
    const auto first = size - lsb - width;
    for (size_t i = 0; i < size; ++i) {
        require(raw[i] < states, "unknown FSDB logic state");
        if (i >= first && i - first < width) out[i - first] = alphabet[raw[i]];
    }
}
}

struct ondas_fsdb {
    ffrObject *file = nullptr;
    ffrTimeBasedVCTrvsHdl cursor = nullptr;
    bool loaded = false, advance = false, view_window = false;
    std::vector<Declaration> declarations;
    std::vector<uint8_t> point_value;
    std::unordered_map<uint64_t, size_t> variables;
    std::unordered_map<unsigned, Datatype> datatypes;
    std::string path, scale, writer, date;
    bool has_writer = false, has_date = false, has_variables = false;
    uint64_t first = 0, last = 0, start_tick = 0, end_tick = 0;
    std::exception_ptr tree_error;
    void end() noexcept {
        // No exceptions may escape cleanup, including during Rust unwinding.
        if (cursor) { try { cursor->ffrFree(); } catch (...) {} cursor = nullptr; }
        if (loaded) { try { file->ffrUnloadSignals(); } catch (...) {} loaded = false; }
        if (file) { try { file->ffrResetSignalList(); } catch (...) {} }
        advance = false;
    }
    ~ondas_fsdb() noexcept {
        end();
        if (file) { try { file->ffrClose(); } catch (...) {} }
    }
};

namespace {
bool_T tree(fsdbTreeCBType type, void *user, void *raw) noexcept {
    auto &reader = *static_cast<ondas_fsdb *>(user);
    if (reader.tree_error) return false;
    try {
        Declaration d;
        switch (type) {
        case FSDB_TREE_CBT_SCOPE: {
            const auto &v = *static_cast<fsdbTreeCBDataScope *>(raw);
            d.data.entry = OFS_SCOPE;
            d.name = text(v.name); d.kind = scope_kind(v.type); d.definition = text(v.module);
            d.data.is_hidden = v.is_hidden_scope;
            break;
        }
        case FSDB_TREE_CBT_RECORD_BEGIN: {
            const auto &v = *static_cast<fsdbTreeCBDataRecordBegin *>(raw);
            d.data.entry = OFS_SCOPE; d.name = text(v.name); d.kind = "record";
            break;
        }
        case FSDB_TREE_CBT_STRUCT_BEGIN: {
            const auto &v = *static_cast<fsdbTreeCBDataStructBegin *>(raw);
            d.data.entry = OFS_SCOPE; d.name = text(v.name);
            d.kind = v.type == FSDB_STRUCT_TYPE_VHDL_RECORD ? "record" :
                (v.type >= FSDB_STRUCT_TYPE_UNPACKED_UNION ? "union" : "struct");
            switch (v.type) {
            case FSDB_STRUCT_TYPE_PACKED_STRUCT:
            case FSDB_STRUCT_TYPE_PACKED_UNION: d.data.packing = 1; break;
            case FSDB_STRUCT_TYPE_UNPACKED_STRUCT:
            case FSDB_STRUCT_TYPE_UNPACKED_UNION:
            case FSDB_STRUCT_TYPE_UNPACKED_TAGGED_UNION: d.data.packing = 2; break;
            case FSDB_STRUCT_TYPE_TAGGED_PACKED_UNION: d.data.packing = 3; break;
            default: break;
            }
            break;
        }
        case FSDB_TREE_CBT_UPSCOPE:
        case FSDB_TREE_CBT_RECORD_END:
        case FSDB_TREE_CBT_STRUCT_END: d.data.entry = OFS_UPSCOPE; break;
        case FSDB_TREE_CBT_VAR:
        case FSDB_TREE_CBT_ENUM_VAR:
        case FSDB_TREE_CBT_PACKED_VAR:
        case FSDB_TREE_CBT_PACKED_COMP_VAR: {
            const auto &v = *static_cast<fsdbTreeCBDataVar *>(raw);
            require(v.u.idcode > 0, "invalid FSDB variable identity");
            d.data.entry = OFS_VAR; d.data.id = uint64_t(v.u.idcode);
            d.name = text(v.name); d.kind = var_kind(v.type);
            d.data.msb = v.lbitnum; d.data.lsb = v.rbitnum;
            d.data.direction = v.direction;
            d.data.is_constant = v.type == FSDB_VT_VCD_PARAMETER || v.type == FSDB_VT_VHDL_CONSTANT;
            encoding(d, v);
            const auto datatype = reader.datatypes.find(d.dt);
            if (datatype != reader.datatypes.end()) {
                d.kind = "enum";
                d.enum_logic = datatype->second.logic;
                const auto width = uint64_t(std::abs(int64_t(v.lbitnum) - v.rbitnum)) + 1;
                if (d.enum_logic && v.bytes_per_bit == FSDB_BYTES_PER_BIT_1B) {
                    require(width <= UINT32_MAX, "enum width exceeds u32");
                    d.data.encoding = OFS_BITS;
                    d.data.width = uint32_t(width);
                    d.data.has_range = width > 1 || v.lbitnum != 0;
                }
            }
            auto existing = reader.variables.find(d.data.id);
            if (existing != reader.variables.end()) {
                const auto &previous = reader.declarations[existing->second];
                require(previous.data.encoding == d.data.encoding && previous.data.width == d.data.width &&
                        previous.dt == d.dt && previous.bytes_per_bit == d.bytes_per_bit,
                        "incompatible alias declarations");
            } else reader.variables.emplace(d.data.id, reader.declarations.size());
            break;
        }
        case FSDB_TREE_CBT_DT_ENUM: {
            const auto &v = *static_cast<fsdbTreeCBDataEnum *>(raw);
            require(!v.numLiteral || v.arrLiteral, "missing FSDB ordinal enum table");
            Datatype type;
            unsigned width = 1;
            while ((uint64_t(1) << width) < v.numLiteral) ++width;
            for (unsigned i = 0; i < v.numLiteral; ++i) {
                require(v.arrLiteral[i] != nullptr, "missing FSDB enum label");
                std::string bits(width, '0');
                for (unsigned bit = 0; bit < width; ++bit)
                    bits[width - 1 - bit] = ((i >> bit) & 1) ? '1' : '0';
                type.variants.emplace_back(std::move(bits), v.arrLiteral[i]);
            }
            reader.datatypes[v.idcode] = std::move(type);
            return true;
        }
        case FSDB_TREE_CBT_DT_ENUM2: {
            const auto &v = *static_cast<fsdbTreeCBDataEnum2 *>(raw);
            const bool logic = v.val_type == FSDB_ENUM_VALUE_TYPE_LOGIC ||
                v.val_type == FSDB_ENUM_VALUE_TYPE_VERILOG_LOGIC || v.val_type == FSDB_ENUM_VALUE_TYPE_VHDL_LOGIC;
            require(logic || v.val_len <= UINT32_MAX / 8, "enum width overflow");
            reader.datatypes[v.idcode] = enumeration(v.name, v.val_type, v.val_len * (logic ? 1 : 8),
                                                    v.literal_count, v.literal_arr, v.val_arr);
            return true;
        }
        case FSDB_TREE_CBT_DT_ENUM3: {
            const auto &v = *static_cast<fsdbTreeCBDataEnum3 *>(raw);
            require(uint64_t(std::abs(int64_t(v.lbitnum) - v.rbitnum)) + 1 == v.val_len,
                    "inconsistent FSDB enum bounds");
            reader.datatypes[v.idcode] = enumeration(v.name, v.val_type, v.val_len,
                                                    v.literal_count, v.literal_arr, v.val_arr);
            return true;
        }
        default: return true;
        }
        reader.declarations.push_back(std::move(d));
        return true;
    } catch (...) { reader.tree_error = std::current_exception(); return false; }
}
}

// The catch sites only normalize exceptions; status failures are checked at each
// SDK call. A generic backend failure must not be labelled file corruption.
#define OFS_CATCH(error, cap) \
    catch (const std::exception &e) { report(error, cap, e.what()); return -1; } \
    catch (...) { report(error, cap, "unknown C++ exception in FSDB Reader"); return -1; }

extern "C" int ondas_fsdb_probe(const char *path, int *is_fsdb, char *error, size_t cap) {
    try {
        std::string filename(path);
        require(!filename.empty(), "empty FSDB path");
        *is_fsdb = ffrObject::ffrIsFSDB(&filename[0]);
        return 0;
    }
    OFS_CATCH(error, cap)
}
extern "C" int ondas_fsdb_open(const char *path, int metadata_only, ondas_fsdb **out, char *error, size_t cap) {
    *out = nullptr;
    try {
        require(ondas_fsdb_dependencies[0] && ondas_fsdb_dependencies[1], "missing SDK dependencies");
        std::unique_ptr<ondas_fsdb> reader(new ondas_fsdb);
        reader->path = path;
        require(!reader->path.empty(), "empty FSDB path");
        reader->file = ffrObject::ffrOpenNonSharedObj(&reader->path[0]);
        require(reader->file != nullptr, "FSDB Reader could not open the file (check file variant and SDK version)");
        auto *file = reader->file;
        ffrFSDBInfo info{};
        reader->view_window = ffrObject::ffrGetFSDBInfo(&reader->path[0], info) == FSDB_RC_SUCCESS
                              && info.is_view_window_available;
        require(file->ffrGetXTagType() == FSDB_XTAG_TYPE_L || file->ffrGetXTagType() == FSDB_XTAG_TYPE_HL,
                "floating FSDB timestamps are unsupported");
        if (!metadata_only) {
            file->ffrSetTreeCBFunc(tree, reader.get());
            if (file->ffrHasDataTypeDef()) {
                uint_T block = 0; // The SDK reads from this block through the last block.
                const auto rc = file->ffrReadDataTypeDefByBlkIdx(block);
                if (reader->tree_error) std::rethrow_exception(reader->tree_error);
                success(rc, "read FSDB datatype definitions");
            }
            const auto rc = file->ffrReadScopeVarTree();
            if (reader->tree_error) std::rethrow_exception(reader->tree_error);
            success(rc, "read FSDB hierarchy");
        }
        // The SDK's maximum idcode is the unique-signal count, available before tree traversal.
        reader->has_variables = metadata_only ? file->ffrGetMaxVarIdcode() != 0
                                              : !reader->variables.empty();
        fsdbTag64 first{}, last{};
        success(file->ffrGetMinFsdbTag64(&first), "read first tick");
        success(file->ffrGetMaxFsdbTag64(&last), "read last tick");
        reader->first = ticks(first); reader->last = ticks(last);
        require(reader->first <= reader->last, "reversed recorded time span");
        reader->scale = text(file->ffrGetScaleUnit());
        reader->has_writer = file->ffrGetSimVersion() != nullptr;
        reader->writer = text(file->ffrGetSimVersion());
        reader->has_date = file->ffrGetSimDate() != nullptr;
        reader->date = text(file->ffrGetSimDate());
        *out = reader.release();
        return 0;
    } OFS_CATCH(error, cap)
}
extern "C" void ondas_fsdb_close(ondas_fsdb *reader) { delete reader; }
extern "C" void ondas_fsdb_metadata(ondas_fsdb *reader, ondas_fsdb_meta *out) {
    *out = {reader->first, reader->last, reader->has_variables,
            reader->scale.c_str(), reader->has_writer ? reader->writer.c_str() : nullptr,
            reader->has_date ? reader->date.c_str() : nullptr};
}
extern "C" size_t ondas_fsdb_decl_count(ondas_fsdb *reader) { return reader->declarations.size(); }
extern "C" void ondas_fsdb_declaration(ondas_fsdb *reader, size_t index, ondas_fsdb_decl *out) {
    const auto &d = reader->declarations[index];
    *out = d.data;
    out->name = d.name.c_str(); out->kind = d.kind.c_str();
    out->definition = d.definition.empty() ? nullptr : d.definition.c_str();
    const auto type = reader->datatypes.find(d.dt);
    if (d.data.entry == OFS_VAR && type != reader->datatypes.end()) {
        out->type_name = type->second.name.empty() ? nullptr : type->second.name.c_str();
        out->enum_count = type->second.variants.size();
    }
}
extern "C" void ondas_fsdb_enum_variant(ondas_fsdb *reader, size_t declaration, size_t variant,
                                       const char **bits, const char **label) {
    const auto &d = reader->declarations[declaration];
    const auto &entry = reader->datatypes.find(d.dt)->second.variants[variant];
    *bits = entry.first.c_str(); *label = entry.second.c_str();
}
extern "C" void ondas_fsdb_end(ondas_fsdb *reader) { reader->end(); }
extern "C" int ondas_fsdb_begin(ondas_fsdb *reader, const uint64_t *ids, size_t count, uint64_t begin, uint64_t end, char *error, size_t cap) {
    try {
        reader->end();
        require(count > 0 && count <= UINT32_MAX, "invalid selected signal count");
        reader->start_tick = begin;
        reader->end_tick = end;
        std::vector<fsdbVarIdcode> selected(ids, ids + count);
        for (auto id : selected) {
            const auto &d = reader->declarations.at(reader->variables.at(id));
            require(d.data.encoding != OFS_EVENT || !reader->file->ffrHasDumpOffRange(),
                    "FSDB event queries with dump-off ranges are unsupported");
            success(reader->file->ffrAddToSignalList(id), "select FSDB signal");
        }
        // A nonzero start requires the engine's exact normalized entering state.
        // Without one, begin is zero. Loading remains flush-session granular.
        if (reader->view_window) {
            fsdbXTag start{}, close{};
            if (reader->file->ffrGetXTagType() == FSDB_XTAG_TYPE_L) {
                start.ltag.L = begin > UINT32_MAX ? UINT32_MAX : uint32_t(begin);
                close.ltag.L = end > UINT32_MAX ? UINT32_MAX : uint32_t(end);
            } else {
                start.hltag.H = uint32_t(begin >> 32);
                start.hltag.L = uint32_t(begin);
                close.hltag.H = uint32_t(end >> 32);
                close.hltag.L = uint32_t(end);
            }
            success(reader->file->ffrResetViewWindow(&start, &close), "set FSDB view window");
        }
        reader->loaded = true;
        success(reader->file->ffrLoadSignals(), "load selected FSDB signals");
        reader->cursor = reader->file->ffrCreateTimeBasedVCTrvsHdl(uint32_t(count), selected.data());
        require(reader->cursor != nullptr, "create FSDB chronological cursor");
        return 0;
    } OFS_CATCH(error, cap)
}
namespace {
struct PointCursor {
    ffrVCTrvsHdl value;
    ~PointCursor() { if (value) { try { value->ffrFree(); } catch (...) {} } }
};
uint64_t point_tick(ffrVCTrvsHdl cursor, unsigned tag_type) {
    fsdbXTag tag{};
    success(cursor->ffrGetXTag(&tag), "read FSDB point tick");
    return tag_type == FSDB_XTAG_TYPE_L ? tag.ltag.L : (uint64_t(tag.hltag.H) << 32) | tag.hltag.L;
}
bool final_bits(ffrVCTrvsHdl cursor, unsigned tag_type, const Declaration &d,
                uint64_t requested, unsigned lsb, unsigned width,
                uint64_t &at, std::vector<uint8_t> &value) {
    if (!cursor->ffrHasIncoreVC()) return false;
    fsdbXTag first{};
    success(cursor->ffrGetMinXTag(&first), "read first FSDB point tick");
    const auto first_tick = tag_type == FSDB_XTAG_TYPE_L ? uint64_t(first.ltag.L) : (uint64_t(first.hltag.H) << 32) | first.hltag.L;
    if (requested < first_tick) return false;
    fsdbXTag last{};
    success(cursor->ffrGetMaxXTag(&last), "read last FSDB point tick");
    const auto last_tick = tag_type == FSDB_XTAG_TYPE_L ? uint64_t(last.ltag.L) : (uint64_t(last.hltag.H) << 32) | last.hltag.L;
    if (requested > last_tick) requested = last_tick;
    fsdbXTag tag{};
    if (tag_type == FSDB_XTAG_TYPE_L) tag.ltag.L = requested > UINT32_MAX ? UINT32_MAX : uint32_t(requested);
    else { tag.hltag.H = uint32_t(requested >> 32); tag.hltag.L = uint32_t(requested); }
    int glitches = 0;
    success(cursor->ffrGotoXTag(&tag, &glitches), "seek FSDB point");
    at = point_tick(cursor, tag_type);
    require(at <= requested, "FSDB point seek moved past requested tick");
    value.resize(width);
    // Finish the aligned tick, including equal-time records exposed separately
    // by some SDK/file combinations. No later tick becomes the sampled state.
    while (true) {
        byte_T *raw = nullptr;
        success(cursor->ffrGetVC(&raw), "read FSDB point value");
        decode_bits(d, raw, cursor->ffrGetByteCount(), lsb, width, value.data());
        if (cursor->ffrGotoNextVC() != FSDB_RC_SUCCESS) break;
        const auto next = point_tick(cursor, tag_type);
        require(next >= at, "backwards FSDB point traversal");
        if (next != at) break;
    }
    return true;
}
}
extern "C" int ondas_fsdb_sample_bits(ondas_fsdb *reader, uint64_t id, uint64_t time,
                                      uint32_t lsb, uint32_t width, ondas_fsdb_value *out,
                                      int *changed, char *error, size_t cap) {
    *out = {}; *changed = 0;
    try {
        require(reader->loaded, "no loaded FSDB point selection");
        const auto &d = reader->declarations.at(reader->variables.at(id));
        require(d.data.encoding == OFS_BITS, "FSDB point fast path requires bits");
        PointCursor cursor{reader->file->ffrCreateVCTrvsHdl(fsdbVarIdcode(id))};
        require(cursor.value != nullptr, "create FSDB point cursor");
        const auto tag_type = reader->file->ffrGetXTagType();
        uint64_t at = 0;
        if (!final_bits(cursor.value, tag_type, d, time, lsb, width, at, reader->point_value)) return 0;
        std::vector<uint8_t> previous;
        // ponytail: a constant projection can walk every preceding tick to prove
        // changed_at is unknown; add an index only if that workload requires it.
        while (at != 0) {
            uint64_t before = 0;
            if (!final_bits(cursor.value, tag_type, d, at - 1, lsb, width, before, previous)) break;
            require(before < at, "FSDB point predecessor did not move backwards");
            if (previous != reader->point_value) { *changed = 1; break; }
            at = before;
        }
        out->id = id; out->tick = at; out->encoding = OFS_BITS;
        out->data = reader->point_value.data(); out->len = reader->point_value.size();
        return 1;
    } OFS_CATCH(error, cap)
}
extern "C" int ondas_fsdb_next(ondas_fsdb *reader, ondas_fsdb_value *out, uint8_t *bits, size_t bits_cap, char *error, size_t cap) {
    try {
        auto *cursor = reader->cursor;
        require(cursor != nullptr, "no active FSDB traversal");
        while (true) {
            if (reader->advance && cursor->ffrGotoNextVC() != FSDB_RC_SUCCESS) return 0;
            fsdbTag64 time{};
            // A newly created chronological cursor may be empty.
            if (cursor->ffrGetXTag(&time) != FSDB_RC_SUCCESS) return 0;
            reader->advance = true;
            const auto tick = ticks(time);
            if (tick > reader->end_tick) return 0;
            // The SDK may include an entering value before the view window.
            // The engine already owns that state; do not replay it as a change.
            if (tick < reader->start_tick) continue;
            fsdbVarIdcode id = 0;
            success(cursor->ffrGetVarIdcode(&id), "read FSDB value identity");
            const auto &d = reader->declarations.at(reader->variables.at(id));
            *out = {}; out->tick = tick; out->id = id; out->encoding = d.data.encoding;
            const auto size = cursor->ffrGetByteCount();
            byte_T *raw = nullptr;
            success(cursor->ffrGetVC(&raw), "read FSDB value");
            // Consume borrowed storage immediately: no more SDK calls until
            // it is converted here or copied by Rust under the same lock.
            require(size != UINT32_MAX && (raw != nullptr || size == 0), "invalid FSDB value buffer");
            if (out->encoding == OFS_EVENT) {
                require(size == 1 && raw != nullptr, "invalid FSDB event storage");
                if (raw[0] == FSDB_BT_VCD_NC) continue; // No-change initialization, not a trigger.
                require(raw[0] == FSDB_BT_VCD_1, "unsupported FSDB event record");
            } else if (out->encoding == OFS_BITS) {
                require(bits != nullptr && size <= bits_cap, "short FSDB bit buffer");
                decode_bits(d, raw, size, 0, d.data.width, bits);
                out->data = bits; out->len = size;
            } else if (out->encoding == OFS_REAL) {
                require((size == 4 && d.bytes_per_bit == FSDB_BYTES_PER_BIT_4B) ||
                        (size == 8 && d.bytes_per_bit == FSDB_BYTES_PER_BIT_8B), "invalid FSDB real storage");
                out->data = raw; out->len = size;
            } else if (out->encoding == OFS_STRING) {
                // For strings GetByteCount describes the SDK index, not the text.
                // GetVC supplies a NUL-terminated byte string owned by the cursor.
                require(raw != nullptr, "null FSDB string");
                out->data = raw; out->len = std::strlen(reinterpret_cast<const char *>(raw));
            } else throw std::runtime_error("unsupported FSDB value class");
            return 1;
        }
    } OFS_CATCH(error, cap)
}
