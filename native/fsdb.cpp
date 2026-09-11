#include "fsdb.h"
#include "ffrAPI.h"
#include <cstdio>
#include <cstring>
#include <exception>
#include <limits>
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
    case FSDB_VT_VCD_EVENT: case FSDB_VT_EVENT_VARIABLE: return "event";
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
struct Declaration {
    ondas_fsdb_decl data{};
    std::string name, kind, definition;
    // Only used inside C++: SDK data type and storage representation.
    unsigned dt = 0, bytes_per_bit = 0;
};
void encoding(Declaration &d, const fsdbTreeCBDataVar &v) {
    auto &out = d.data;
    const auto width = uint64_t(std::abs(int64_t(v.lbitnum) - v.rbitnum)) + 1;
    d.dt = v.dtidcode;
    d.bytes_per_bit = v.bytes_per_bit;
    out.encoding = OFS_UNSUPPORTED;
    if (v.type == FSDB_VT_VCD_EVENT || v.type == FSDB_VT_EVENT_VARIABLE || d.dt == FSDB_DT_SV_EVENT) {
        out.encoding = OFS_EVENT;
    } else if (v.type == FSDB_VT_STRING || d.dt == FSDB_DT_SV_STRING || d.dt == FSDB_DT_VHDL_STRING) {
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
}

struct ondas_fsdb {
    ffrObject *file = nullptr;
    ffrTimeBasedVCTrvsHdl cursor = nullptr;
    bool loaded = false, advance = false, eof = false;
    std::vector<Declaration> declarations;
    std::unordered_map<uint64_t, size_t> variables;
    std::vector<uint8_t> buffer;
    std::string path, scale, writer, date;
    bool has_writer = false, has_date = false;
    uint64_t first = 0, last = 0;
    std::exception_ptr tree_error;
    void end() noexcept {
        // No exceptions may escape cleanup, including during Rust unwinding.
        if (cursor) { try { cursor->ffrFree(); } catch (...) {} cursor = nullptr; }
        if (loaded) { try { file->ffrUnloadSignals(); } catch (...) {} loaded = false; }
        if (file) { try { file->ffrResetSignalList(); } catch (...) {} }
        advance = eof = false;
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
        case FSDB_TREE_CBT_VAR: {
            const auto &v = *static_cast<fsdbTreeCBDataVar *>(raw);
            require(v.u.idcode > 0, "invalid FSDB variable identity");
            d.data.entry = OFS_VAR; d.data.id = uint64_t(v.u.idcode);
            d.name = text(v.name); d.kind = var_kind(v.type);
            d.data.msb = v.lbitnum; d.data.lsb = v.rbitnum;
            d.data.direction = v.direction;
            d.data.is_constant = v.type == FSDB_VT_VCD_PARAMETER || v.type == FSDB_VT_VHDL_CONSTANT;
            encoding(d, v);
            auto existing = reader.variables.find(d.data.id);
            if (existing != reader.variables.end()) {
                const auto &previous = reader.declarations[existing->second];
                require(previous.data.encoding == d.data.encoding && previous.data.width == d.data.width &&
                        previous.dt == d.dt && previous.bytes_per_bit == d.bytes_per_bit,
                        "incompatible alias declarations");
            } else reader.variables.emplace(d.data.id, reader.declarations.size());
            break;
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
extern "C" int ondas_fsdb_open(const char *path, ondas_fsdb **out, char *error, size_t cap) {
    *out = nullptr;
    try {
        require(ondas_fsdb_dependencies[0] && ondas_fsdb_dependencies[1], "missing SDK dependencies");
        std::unique_ptr<ondas_fsdb> reader(new ondas_fsdb);
        reader->path = path;
        require(!reader->path.empty(), "empty FSDB path");
        reader->file = ffrObject::ffrOpenNonSharedObj(&reader->path[0]);
        require(reader->file != nullptr, "FSDB Reader could not open the file (check file variant and SDK version)");
        auto *file = reader->file;
        require(file->ffrGetXTagType() == FSDB_XTAG_TYPE_L || file->ffrGetXTagType() == FSDB_XTAG_TYPE_HL,
                "floating FSDB timestamps are unsupported");
        file->ffrSetTreeCBFunc(tree, reader.get());
        const auto rc = file->ffrReadScopeVarTree();
        if (reader->tree_error) std::rethrow_exception(reader->tree_error);
        success(rc, "read FSDB hierarchy");
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
    *out = {reader->first, reader->last, reader->scale.c_str(),
            reader->has_writer ? reader->writer.c_str() : nullptr,
            reader->has_date ? reader->date.c_str() : nullptr};
}
extern "C" size_t ondas_fsdb_decl_count(ondas_fsdb *reader) { return reader->declarations.size(); }
extern "C" void ondas_fsdb_declaration(ondas_fsdb *reader, size_t index, ondas_fsdb_decl *out) {
    const auto &d = reader->declarations[index];
    *out = d.data;
    out->name = d.name.c_str(); out->kind = d.kind.c_str();
    out->definition = d.definition.empty() ? nullptr : d.definition.c_str();
}
extern "C" void ondas_fsdb_end(ondas_fsdb *reader) { reader->end(); }
extern "C" int ondas_fsdb_begin(ondas_fsdb *reader, const uint64_t *ids, size_t count, char *error, size_t cap) {
    try {
        reader->end();
        require(count > 0 && count <= UINT32_MAX, "invalid selected signal count");
        std::vector<fsdbVarIdcode> selected(ids, ids + count);
        for (auto id : selected) success(reader->file->ffrAddToSignalList(id), "select FSDB signal");
        reader->loaded = true;
        success(reader->file->ffrLoadSignals(), "load selected FSDB signals");
        reader->cursor = reader->file->ffrCreateTimeBasedVCTrvsHdl(uint32_t(count), selected.data());
        require(reader->cursor != nullptr, "create FSDB chronological cursor");
        return 0;
    } OFS_CATCH(error, cap)
}
extern "C" int ondas_fsdb_next(ondas_fsdb *reader, ondas_fsdb_value *out, char *error, size_t cap) {
    try {
        auto *cursor = reader->cursor;
        require(cursor != nullptr, "no active FSDB traversal");
        if (reader->eof) return 0;
        if (reader->advance && cursor->ffrGotoNextVC() != FSDB_RC_SUCCESS) { reader->eof = true; return 0; }
        fsdbTag64 time{};
        // A newly created chronological cursor may be empty.
        if (cursor->ffrGetXTag(&time) != FSDB_RC_SUCCESS) { reader->eof = true; return 0; }
        reader->advance = true;
        fsdbVarIdcode id = 0;
        success(cursor->ffrGetVarIdcode(&id), "read FSDB value identity");
        const auto &d = reader->declarations.at(reader->variables.at(id));
        *out = {}; out->tick = ticks(time); out->id = id; out->encoding = d.data.encoding;
        if (out->encoding == OFS_EVENT) return 1;
        byte_T *raw = nullptr;
        success(cursor->ffrGetVC(&raw), "read FSDB value");
        const auto size = cursor->ffrGetByteCount();
        require(size != UINT32_MAX && (raw != nullptr || size == 0), "invalid FSDB value buffer");
        if (out->encoding == OFS_BITS) {
            require(size == d.data.width, "FSDB vector width mismatch");
            const char *alphabet = "01xz";
            size_t states = 4;
            if (d.dt == FSDB_DT_VHDL_BOOLEAN || d.dt == FSDB_DT_VHDL_BIT || d.dt == FSDB_DT_VHDL_BIT_VECTOR) {
                alphabet = "01"; states = 2;
            } else if (d.dt >= FSDB_DT_VHDL_STD_ULOGIC && d.dt <= FSDB_DT_VHDL_SIGNED) {
                alphabet = "ux01zwlh-"; states = 9;
            }
            reader->buffer.resize(size);
            for (size_t i = 0; i < size; ++i) {
                require(raw[i] < states, "unknown FSDB logic state");
                reader->buffer[i] = alphabet[raw[i]];
            }
            out->data = reader->buffer.data(); out->len = size;
        } else if (out->encoding == OFS_REAL) {
            if (size == sizeof(float) && d.bytes_per_bit == FSDB_BYTES_PER_BIT_4B) {
                float value; std::memcpy(&value, raw, sizeof value); out->real = value;
            } else {
                require(size == sizeof(double) && d.bytes_per_bit == FSDB_BYTES_PER_BIT_8B, "invalid FSDB real storage");
                std::memcpy(&out->real, raw, sizeof out->real);
            }
        } else if (out->encoding == OFS_STRING) {
            out->data = raw; out->len = size;
        } else throw std::runtime_error("unsupported FSDB value class");
        return 1;
    } OFS_CATCH(error, cap)
}
