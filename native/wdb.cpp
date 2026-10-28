#include "wdb.h"
#include "wdb_abi.hpp"
#include <algorithm>
#include <cstdint>
#include <cstring>
#include <cstdlib>
#include <cxxabi.h>
#include <dlfcn.h>
#include <limits>
#include <map>
#include <memory>
#include <queue>
#include <stdexcept>
#include <string_view>
#include <tuple>
#include <unordered_map>

namespace {
static_assert(sizeof(ondas_wdb_meta) == 16 && offsetof(ondas_wdb_meta, precision) == 8);
static_assert(sizeof(ondas_wdb_decl) == 128 && offsetof(ondas_wdb_decl, msb) == 56 &&
              offsetof(ondas_wdb_decl, reader_name) == 80 && offsetof(ondas_wdb_decl, base_len) == 120);
static_assert(sizeof(ondas_wdb_record) == 40 && offsetof(ondas_wdb_record, data) == 24 &&
              offsetof(ondas_wdb_record, len) == 32);
constexpr size_t no_parent = std::numeric_limits<size_t>::max();
constexpr size_t maximum_storage = 16 * 1024 * 1024;
struct Unsupported : std::runtime_error { using std::runtime_error::runtime_error; };
struct LayoutUnsupported : Unsupported { using Unsupported::Unsupported; };

template<class T> T symbol(const std::string& name) {
    auto address = dlsym(RTLD_DEFAULT, name.c_str());
    if (!address) throw Unsupported("reader is missing ABI symbol " + name);
    return reinterpret_cast<T>(address);
}
void selected_library(const char* name, const char* expected) {
    Dl_info info{};
    if (!dladdr(symbol<const void*>(name), &info) || !info.dli_fname)
        throw Unsupported("cannot identify loaded reader library");
    std::unique_ptr<char, decltype(&free)> actual(realpath(info.dli_fname, nullptr), &free);
    if (!actual || std::strcmp(actual.get(), expected))
        throw Unsupported("loaded reader library differs from the build-time selection");
}
const void* pointer(const void* object, size_t offset) {
    if (!object) throw std::runtime_error("null native owner");
    return *reinterpret_cast<const void* const*>(static_cast<const char*>(object) + offset);
}
uint32_t word(const void* object, size_t offset) {
    if (!object) throw std::runtime_error("null native owner");
    return *reinterpret_cast<const uint32_t*>(static_cast<const char*>(object) + offset);
}
const std::string& class_name(const void* object) {
    if (!object) throw Unsupported("null native type");
    auto table = *static_cast<void* const*>(object);
    // Linked-library vtables are immutable; Rust holds the SDK mutex here.
    static std::map<const void*, std::string> names;
    if (auto found = names.find(table); found != names.end()) return found->second;
    Dl_info info{};
    if (!dladdr(table, &info) || !info.dli_sname)
        throw Unsupported("unidentified reader ABI class");
    int status = 0;
    char* demangled = abi::__cxa_demangle(info.dli_sname, nullptr, nullptr, &status);
    std::string result = demangled ? demangled : info.dli_sname;
    free(demangled);
    return names.emplace(table, std::move(result)).first->second;
}
void text(const std::string& value) {
    if (value.find('\0') != std::string::npos)
        throw Unsupported("embedded zero in native declaration text");
}
bool predefined_name(std::string_view name, std::string_view expected) {
    if (name.size() != expected.size()) return false;
    for (size_t i = 0; i < name.size(); ++i) {
        auto c = name[i];
        if (c >= 'A' && c <= 'Z') c += 'a' - 'A';
        if (c != expected[i]) return false;
    }
    return true;
}
uint32_t logic_domain(const SimBridge::IType* type) {
    auto owner = pointer(type, 8);
    auto name = vcall<std::string>(type, 4);
    if (predefined_name(name, "bit")) return 1;
    if (name == "logic" || name == "reg") return 2;
    return word(owner, 0x30) == 0 ? 3 : 0;
}
struct Constraints { int32_t left, right, direction; };
struct Variant { std::string bits, label; };
struct TypeInfo {
    std::string kind, name;
    std::string_view native_class, element_class;
    const SimBridge::IType* element = nullptr;
    unsigned packing = 0, domain = 0;
    bool characters = false;
};
using Identity = std::tuple<long long, unsigned, unsigned, unsigned, unsigned, uintptr_t, int64_t, int64_t>;
struct Declaration {
    const SimBridge::IObject* object = nullptr;
    const SimBridge::IType* type = nullptr;
    const void* character = nullptr;
    std::shared_ptr<const RTTI::FullConstraints> constraints;
    std::string name;
    std::shared_ptr<const std::string> reader_name;
    const char* kind = nullptr;
    const char* type_name = nullptr;
    std::vector<Variant> variants;
    size_t parent = no_parent;
    uint64_t id = 0;
    uint32_t encoding = 0, width = 0, domain = 0, packing = 0, direction = 0;
    bool scope = false, has_range = false, constant = false, ordinal_enum = false, integer = false, grouped_bits = false;
    uint32_t name_syntax = 0;
    bool qualified = true;
    size_t base_len = 0;
    int64_t msb = 0, lsb = 0;
    long long address = -1;
    uint32_t length = 0, offset = 0;
};
Identity identity(const Declaration& d) {
    return {d.address, d.length, d.offset, d.width, d.encoding,
            reinterpret_cast<uintptr_t>(d.type), d.msb, d.lsb};
}
struct ByteValue {
    int32_t kind;
    unsigned char value, padding[3];
};
struct TimeByteValue { int64_t tick; ByteValue byte; };
struct TimeByteValues { int64_t tick = 0; std::vector<ByteValue> bytes; };
static_assert(sizeof(ByteValue) == 8 && sizeof(TimeByteValue) == 16 && sizeof(TimeByteValues) == 32);
struct IndexBatch {
    void* manager;
    std::vector<void*> created;
    bool committed = false;
    long long (*address)(const void*) = symbol<long long(*)(const void*)>(
        "_ZNK9SimBridge14WdbScalarBTree10getAddressEv");
    void (*remove)(void*, long long, long long) = symbol<void(*)(void*, long long, long long)>(
        "_ZN9SimBridge22WdbAddressBTreeManager16deleteByteBTreesExx");
    explicit IndexBatch(void* source) : manager(source) {}
    void deduplicate() {
        std::sort(created.begin(), created.end(), std::less<void*>{});
        created.erase(std::unique(created.begin(), created.end()), created.end());
    }
    ~IndexBatch() {
        // A failed scan must not leave a partial SDK index available for reuse.
        if (!committed) {
            deduplicate();
            for (auto tree : created) remove(manager, address(tree), 1);
        }
    }
    void add(long long address, unsigned length, std::vector<void*>& trees) {
        static auto create = symbol<void(*)(void*, long long, long long, std::vector<void*>&, std::vector<void*>&)>(
            "_ZN9SimBridge22WdbAddressBTreeManager21getOrCreateByteBTreesExxRSt6vectorIPNS_14WdbScalarBTreeESaIS3_EES6_");
        create(manager, address, length, trees, created);
        if (trees.size() != length) throw std::runtime_error("native byte index width disagrees with extent");
    }
    void load() {
        deduplicate();
        static auto scan = symbol<void(*)(void*, const std::vector<void*>&, void*)>(
            "_ZN9SimBridge22WdbAddressBTreeManager15scanWdbDatabaseERKSt6vectorIPNS_14WdbScalarBTreeESaIS3_EEPN4ISIM10IGuiStatusE");
        if (!created.empty()) scan(manager, created, nullptr);
        committed = true;
    }
};
struct CursorDelete {
    void operator()(void* iterator) const { if (iterator) vcall<void>(iterator, 1); }
};
struct ByteCursor {
    std::unique_ptr<void, CursorDelete> iterator;
    std::vector<std::pair<size_t, size_t>> targets;
    TimeByteValue current{};
    bool valid = false;
    ByteCursor(const void* tree, long long start, long long last, long long end) {
        static auto find = symbol<void*(*)(const void*, long long, long long, long long, long long)>(
            "_ZNK9SimBridge14WdbScalarBTree4findExxxx");
        iterator.reset(find(tree, start, 0, last, end));
        if (!iterator) throw std::runtime_error("null native byte iterator");
        refresh();
        while (valid && current.tick < start) advance();
    }
    void refresh() {
        static auto present = symbol<bool(*)(const void*)>(
            "_ZNK9SimBridge14WdbScalarBTree8IteratorcvbEv");
        static auto read = symbol<TimeByteValue(*)(const void*)>(
            "_ZNK9SimBridge14WdbScalarBTree8IteratordeEv");
        valid = present(iterator.get());
        if (valid) {
            current = read(iterator.get());
            if (current.tick < 0) throw std::runtime_error("negative native byte time");
        }
    }
    void advance() {
        auto previous = current.tick;
        static auto next = symbol<void*(*)(void*)>(
            "_ZN9SimBridge14WdbScalarBTree8IteratorppEv");
        next(iterator.get());
        refresh();
        if (valid && current.tick <= previous) throw std::runtime_error("non-increasing native byte history");
    }
};
struct Selected {
    const Declaration* declaration;
    std::vector<void*> trees;
    std::vector<unsigned char> bytes, known, value;
    size_t unknown;
    bool observed = false, dirty = false;
    explicit Selected(const Declaration& d) : declaration(&d), bytes(d.length), known(d.length), unknown(d.length) {}
};
using QueueItem = std::pair<long long, size_t>;
struct Traversal {
    uint64_t end;
    std::vector<Selected> selected;
    std::vector<std::unique_ptr<ByteCursor>> cursors;
    std::priority_queue<QueueItem, std::vector<QueueItem>, std::greater<QueueItem>> queue;
    bool pending = false;
    size_t selection = 0;
    long long tick = 0;
    std::vector<unsigned char> output;
};
struct Reader {
    SimBridge::DataReader native;
    ondas_wdb_meta meta{};
    std::vector<Declaration> declarations;
    size_t scope_count = 0;
    std::unordered_map<const SimBridge::IType*, TypeInfo> types;
    const SimBridge::DataConversion* conversion = nullptr;
    const void* common = nullptr;
    void* index_manager = nullptr;
    std::unique_ptr<Traversal> traversal;
    explicit Reader(const char* path) : native(static_cast<SimBridge::WdbOpenMode>(1), true, path, path, {}, 0) {
        auto last = native.getLatestTime();
        if (last < 0 || last == std::numeric_limits<long long>::max())
            throw Unsupported("native reader cannot represent the recording time bound");
        meta.last = static_cast<uint64_t>(last);
        meta.precision = native.getPrecision();
    }
    TypeInfo& names(const SimBridge::IType* type) {
        auto found = types.find(type);
        if (found != types.end()) return found->second;
        TypeInfo info;
        info.kind = vcall<std::string>(type, 3);
        info.name = vcall<std::string>(type, 4);
        text(info.kind); text(info.name);
        return types.emplace(type, std::move(info)).first->second;
    }
    const TypeInfo& type_info(const SimBridge::IType* type) {
        auto& info = names(type);
        if (!info.native_class.empty()) return info;
        auto owner = pointer(type, 8);
        info.native_class = class_name(owner);
        if (info.native_class == "vtable for SimBridge::ArrayTypeObj") {
            info.packing = word(owner, 0x4c);
            static auto inner_type = symbol<const SimBridge::IType*(*)(const void*, bool)>(
                "_ZNK9SimBridge12ArrayTypeObj20getInnerElementITypeEb");
            info.element = inner_type(owner, false);
            info.element_class = class_name(pointer(info.element, 8));
            if (info.element_class == "vtable for SimBridge::BitTypeObj") info.domain = 1;
            else if (info.element_class == "vtable for SimBridge::LogicTypeObj")
                info.domain = logic_domain(info.element);
            info.characters = info.packing == 0 && info.element_class == "vtable for SimBridge::EnumTypeObj" &&
                              predefined_name(vcall<std::string>(info.element, 4), "character");
        } else if (info.native_class == "vtable for SimBridge::LogicTypeObj") {
            info.domain = predefined_name(info.name, "bit") ? 1 :
                (info.name == "logic" || info.name == "reg") ? 2 : word(owner, 0x30) == 0 ? 3 : 0;
        }
        return info;
    }
    bool raw(const Selected& selected, long long time, std::vector<unsigned char>& bytes,
             long long& tick, std::vector<unsigned char>* known = nullptr) const {
        static auto get = symbol<void(*)(const void*, long long, const std::vector<void*>&, TimeByteValues&)>(
            "_ZNK9SimBridge15WdbCommonReader13getValueEventExRKSt6vectorIPNS_14WdbScalarBTreeESaIS3_EERNS_17WdbTimeByteValuesE");
        TimeByteValues state;
        get(common, time, selected.trees, state);
        if (state.bytes.size() != selected.declaration->length || state.tick < 0 || state.tick > time)
            throw std::runtime_error("invalid native point extent or time");
        bytes.resize(state.bytes.size());
        bool complete = true;
        for (size_t i = 0; i < bytes.size(); ++i) {
            const auto byte = state.bytes[i];
            if (byte.kind != 0 && byte.kind != 1)
                throw std::runtime_error("native point returned a non-value record");
            bytes[i] = byte.value;
            if (known) (*known)[i] = byte.kind == 1;
            complete &= byte.kind == 1;
        }
        tick = state.tick;
        return complete;
    }
    void decode(const Declaration& d, const unsigned char* bytes, std::vector<unsigned char>& output) const {
        if (d.encoding == 3) {
            output.resize(d.width);
            static auto character_value = symbol<void(*)(const void*, const SimBridge::BitIndex<const unsigned char*>&, SimBridge::Value&)>(
                "_ZNK9SimBridge11EnumTypeObj16rawMemoryToValueERKNS_8BitIndexIPKhEERNS_5ValueE");
            for (size_t i = 0; i < output.size(); ++i) {
                SimBridge::Value value;
                SimBridge::BitIndex<const unsigned char*> input{bytes + d.offset / 8 + i, 0};
                character_value(d.character, input, value);
                if ((value.kind != 1 && value.kind != 2) || value.payload.integer > 255)
                    throw std::runtime_error("invalid native character code");
                output[i] = static_cast<unsigned char>(value.payload.integer);
            }
            return;
        }
        if (d.encoding == 2) {
            if (d.offset % 8 || d.width != 8)
                throw Unsupported("unqualified native real storage");
            auto source = bytes + d.offset / 8;
            output.resize(8);
            std::memcpy(output.data(), source, 8);
            return;
        }
        if (d.encoding != 1) throw Unsupported("selected native value layout is not qualified");
        if (d.ordinal_enum || d.integer) {
            if (d.offset % 8 || d.width > (d.integer ? 64u : 32u))
                throw Unsupported("unqualified native integral offset or width");
            SimBridge::Value ordinal;
            SimBridge::BitIndex<const unsigned char*> input{bytes + d.offset / 8, 0};
            if (d.integer) {
                static auto convert = symbol<void(*)(const void*, const SimBridge::BitIndex<const unsigned char*>&, SimBridge::Value&)>(
                    "_ZNK9SimBridge10IntTypeObj16rawMemoryToValueERKNS_8BitIndexIPKhEERNS_5ValueE");
                convert(pointer(d.type, 8), input, ordinal);
                if (ordinal.kind < 2 || ordinal.kind > 5) throw std::runtime_error("invalid native integer code");
            } else {
                static auto convert = symbol<void(*)(const void*, const SimBridge::BitIndex<const unsigned char*>&, SimBridge::Value&)>(
                    "_ZNK9SimBridge11EnumTypeObj16rawMemoryToValueERKNS_8BitIndexIPKhEERNS_5ValueE");
                convert(pointer(d.type, 8), input, ordinal);
                if (ordinal.kind != 1 && ordinal.kind != 2) throw std::runtime_error("invalid native enum code");
            }
            auto code = ordinal.payload.integer;
            output.resize(d.width);
            for (unsigned bit = 0; bit < d.width; ++bit) output[d.width - 1 - bit] = '0' + ((code >> bit) & 1);
            return;
        }
        SimBridge::ValueFormat format;
        SimBridge::Value value;
        conversion->formatRawMemory({bytes, d.offset}, d.type, d.constraints.get(), format, false, value);
        if (value.kind != 7 || !value.payload.text)
            throw std::runtime_error("native formatter did not produce bit text");
        output.clear();
        // The formatter groups array dimensions in HDL order, with commas
        // and nested assignment-pattern punctuation.
        for (auto text = value.payload.text; *text; ++text) {
            auto c = *text;
            if (d.grouped_bits && (c == ',' || c == '\'' || c == '{' || c == '}')) continue;
            output.push_back(static_cast<unsigned char>(c));
        }
        if (output.size() != d.width) throw std::runtime_error("native bit width disagrees with declaration");
        constexpr std::string_view supported = "01xzhwlu-";
        for (auto& bit : output) {
            if (bit >= 'A' && bit <= 'Z') bit += 'a' - 'A';
            if (supported.find(bit) == std::string_view::npos)
                throw Unsupported("native value is not a supported logic representation");
        }
    }
};

void describe_type(Reader& reader, Declaration& d) {
    auto data = pointer(d.object, 8);
    auto declaration_kind = vcall<unsigned>(pointer(data, 0x58), 2);
    switch (declaration_kind) {
        case 0x1011: d.direction = 1; break;
        case 0x2011: d.direction = 2; break;
        case 0x4011: d.direction = 3; break;
        case 0x8011: d.direction = 4; break;
        case 0x10011: d.direction = 5; break;
        default: break;
    }
    d.constant = declaration_kind == 0x3021 || declaration_kind == 0x4021 || declaration_kind == 0x5021;
    d.type = vcall<const SimBridge::IType*>(data, 27, false);
    auto named = vcall<const SimBridge::IType*>(data, 27, true);
    const auto& type = reader.type_info(d.type);
    d.kind = type.kind.c_str();
    const auto& type_name = reader.names(named).name;
    d.type_name = type_name.empty() ? nullptr : type_name.c_str();
    auto owner = pointer(d.type, 8);
    auto native_class = type.native_class;
    auto packing = type.packing;
    auto element = type.element;
    auto element_class = type.element_class;
    auto characters = type.characters;
    if (native_class == "vtable for SimBridge::ArrayTypeObj") {
        d.domain = type.domain;
        d.packing = packing == 2 ? 1 : packing == 1 ? 2 : 0;
        const bool logic_elements = element_class == "vtable for SimBridge::LogicTypeObj" ||
                                    element_class == "vtable for SimBridge::BitTypeObj";
        if (!((packing == 2) || (packing == 1 && logic_elements) ||
              (packing == 0 && (element_class == "vtable for SimBridge::LogicTypeObj" || characters)))) return;
    } else if (native_class != "vtable for SimBridge::FloatTypeObj" &&
               native_class != "vtable for SimBridge::LogicTypeObj" &&
               native_class != "vtable for SimBridge::IntTypeObj" &&
               native_class != "vtable for SimBridge::BitTypeObj" &&
               native_class != "vtable for SimBridge::DiscreteEnumArrayTypeObj" &&
               native_class != "vtable for SimBridge::DiscreteEnumScalarTypeObj" &&
               native_class != "vtable for SimBridge::EnumTypeObj") {
        // Unsupported classes need no value constraints. In particular, the
        // SDK can recurse through an entire cyclic class graph when asked.
        return;
    }
    auto storage_width = vcall<unsigned long long>(data, 54);
    if (storage_width > maximum_storage * 8) throw LayoutUnsupported("native value exceeds storage limit");
    if (native_class == "vtable for SimBridge::IntTypeObj") {
        // The native integral converter reads this byte size.
        auto size = word(owner, 0x44);
        if ((size != 1 && size != 2 && size != 4 && size != 8) || storage_width != size * 8)
            throw LayoutUnsupported("unqualified native integer storage");
        d.integer = true;
    }
    unsigned count = 0;
    d.constraints = vcall<std::shared_ptr<const RTTI::FullConstraints>>(data, 52, &count);
    Constraints top{};
    // The native range getter copies these three words from FullConstraints.
    if (d.constraints) std::memcpy(&top, d.constraints.get(), sizeof(top));
    auto native_kind = vcall<int>(d.object, 2);
    if (native_class == "vtable for SimBridge::FloatTypeObj") {
        // FloatTypeObj's native encoder writes binary64. The generic bit metric
        // can report 32 for both real and shortreal declarations.
        d.encoding = 2; d.width = 8;
    }
    else if (native_class == "vtable for SimBridge::LogicTypeObj") {
        d.encoding = 1; d.width = 1; d.domain = type.domain;
    } else if (native_class == "vtable for SimBridge::ArrayTypeObj") {
        if (characters && count == 1 && d.constraints &&
            (top.direction == -1 || top.direction == 1)) {
            auto length = static_cast<uint64_t>(std::max<int64_t>(top.left, top.right) -
                                               std::min<int64_t>(top.left, top.right)) + 1;
            if (length > maximum_storage || storage_width != length * 8)
                throw LayoutUnsupported("unqualified native character array");
            d.encoding = 3; d.width = static_cast<uint32_t>(length);
            d.character = pointer(element, 8);
        }
        if (packing == 1 || packing == 2 || (packing == 0 && element_class == "vtable for SimBridge::LogicTypeObj")) {
            if (!d.constraints || (top.direction != -1 && top.direction != 1))
                throw LayoutUnsupported("unqualified native array constraints");
            d.msb = top.left; d.lsb = top.right; d.has_range = true;
            auto width = static_cast<uint64_t>(std::max(d.msb, d.lsb) - std::min(d.msb, d.lsb)) + 1;
            if (width > std::numeric_limits<uint32_t>::max()) throw LayoutUnsupported("native logic width exceeds API");
            d.width = packing == 0 ? static_cast<uint32_t>(storage_width / 8) : static_cast<uint32_t>(storage_width);
            if (d.width != width || packing == 1) d.has_range = false;
            d.encoding = 1;
            d.grouped_bits = true;
        }
    } else if (native_class == "vtable for SimBridge::IntTypeObj" ||
               native_class == "vtable for SimBridge::BitTypeObj" ||
               native_class == "vtable for SimBridge::DiscreteEnumArrayTypeObj" ||
               native_class == "vtable for SimBridge::DiscreteEnumScalarTypeObj" ||
               native_class == "vtable for SimBridge::EnumTypeObj") {
        d.encoding = 1; d.width = static_cast<uint32_t>(storage_width);
        if (native_class == "vtable for SimBridge::BitTypeObj") d.domain = 1;
    }
    const bool discrete_enum = native_class == "vtable for SimBridge::DiscreteEnumArrayTypeObj" ||
                               native_class == "vtable for SimBridge::DiscreteEnumScalarTypeObj";
    if (discrete_enum && word(owner, 0x30) > 1)
        throw LayoutUnsupported("unqualified native enum representation");
    if (discrete_enum && word(owner, 0x30) == 0) {
        if (storage_width % 8) throw LayoutUnsupported("unqualified native enum storage width");
        d.width = static_cast<uint32_t>(storage_width / 8);
    }
    d.ordinal_enum = native_class == "vtable for SimBridge::EnumTypeObj";
    if (d.encoding == 0) return;
    static auto memory_range = symbol<std::pair<long long, unsigned>(*)(const void*)>("_ZNK9SimBridge7DataObj15getMemAddrRangeEv");
    auto range = memory_range(data);
    d.address = range.first; d.length = range.second; d.offset = word(data, 0x78);
    if ((d.encoding == 2 || d.encoding == 3 || d.integer) && d.offset % 8)
        throw LayoutUnsupported("unqualified native byte offset");
    if (d.length > maximum_storage) throw LayoutUnsupported("native extent exceeds storage limit");
    auto extent = static_cast<uint64_t>(d.offset) + (d.encoding == 2 ? 64 : storage_width);
    auto required = d.encoding != 2 && word(owner, 0x30) == 1 ? ((extent + 31) / 32) * 8 : (extent + 7) / 8;
    if (required > d.length) throw Unsupported("native type exceeds its storage extent");
    if (d.encoding == 1 && d.width == 0) throw LayoutUnsupported("zero native bit width");
    if (native_kind == 5 && !d.has_range && d.encoding == 1) {
        if (d.constraints && (top.direction == -1 || top.direction == 1) &&
            static_cast<uint64_t>(std::max<int64_t>(top.left, top.right) -
                                  std::min<int64_t>(top.left, top.right)) + 1 == d.width) {
            d.msb = top.left; d.lsb = top.right; d.has_range = true;
        }
    }
    if (d.ordinal_enum) {
        auto begin = static_cast<const std::string*>(pointer(owner, 0x40));
        auto end = static_cast<const std::string*>(pointer(owner, 0x48));
        if (!begin || end < begin) throw Unsupported("invalid native ordinal enum table");
        if (end - begin > 1048576 || d.width > 32)
            throw LayoutUnsupported("native ordinal enum exceeds supported bounds");
        static auto code_for = symbol<int(*)(const void*, const std::string&)>(
            "_ZNK9SimBridge11EnumTypeObj18getEnumeratorIndexERKNSt7__cxx1112basic_stringIcSt11char_traitsIcESaIcEEE");
        for (auto entry = begin; entry != end; ++entry) {
            text(*entry);
            int code = code_for(owner, *entry);
            if (code < 0) throw std::runtime_error("native enum label has no code");
            std::string bits(d.width, '0');
            for (unsigned bit = 0; bit < d.width; ++bit) bits[d.width - 1 - bit] = '0' + ((static_cast<unsigned>(code) >> bit) & 1);
            auto label = *entry;
            // Native CHARACTER graphic literals use one Latin-1 byte.
            if (predefined_name(type.name, "character") && label.size() == 1 &&
                static_cast<unsigned char>(label[0]) >= 128) {
                auto character = static_cast<unsigned char>(label[0]);
                label = {static_cast<char>(0xc0 | (character >> 6)), static_cast<char>(0x80 | (character & 0x3f))};
            }
            d.variants.push_back({std::move(bits), std::move(label)});
        }
    } else if (native_class == "vtable for SimBridge::DiscreteEnumArrayTypeObj" ||
        native_class == "vtable for SimBridge::DiscreteEnumScalarTypeObj") {
        struct Entry { std::string label; const unsigned char* bytes; uint64_t padding; };
        static_assert(sizeof(Entry) == 48);
        auto cell_size = word(owner, 0x40);
        auto expected_cell = word(owner, 0x30) == 1 ? ((storage_width + 31) / 32) * 8 : storage_width / 8;
        if (cell_size != expected_cell || cell_size == 0)
            throw LayoutUnsupported("unqualified native enum cell storage");
        auto begin = static_cast<const Entry*>(pointer(owner, 0x48));
        auto end = static_cast<const Entry*>(pointer(owner, 0x50));
        if (!begin || end < begin) throw Unsupported("invalid native enum table");
        if (end - begin > 1048576) throw LayoutUnsupported("native enum table exceeds supported bounds");
        Declaration cell = d;
        cell.offset = 0; cell.length = cell_size;
        for (auto entry = begin; entry != end; ++entry) {
            if (!entry->bytes) throw std::runtime_error("null native enum cell");
            text(entry->label);
            std::vector<unsigned char> bits;
            reader.decode(cell, entry->bytes, bits);
            d.variants.push_back({std::string(bits.begin(), bits.end()), entry->label});
        }
    }
}

void describe(Reader& reader, Declaration& declaration) {
    try { describe_type(reader, declaration); }
    catch (const LayoutUnsupported&) {
        // Unpacked members remain accessible when only the aggregate value
        // exceeds the value-layout qualifications.
        declaration.qualified = declaration.packing == 2;
        declaration.encoding = 0;
        declaration.width = 0;
        declaration.domain = 0;
        declaration.has_range = false;
        declaration.variants.clear();
    }
}

void import(Reader& reader) {
    auto manager = reader.native.getDataObjMgr();
    reader.conversion = reinterpret_cast<const SimBridge::DataConversion*>(static_cast<const char*>(manager) + 0x1c0);
    reader.common = pointer(manager, 0x30);
    if (class_name(reader.common) != "vtable for SimBridge::WdbStaticReader")
        throw Unsupported("native recording reader is not a supported simulation reader");
    auto database_class = class_name(pointer(reader.common, 8));
    bool supported_database = false;
    for (auto name : {"WdbSingleBucketEventDatabase", "WdbMultipleBucketBtreeDatabase", "WdbMultipleBucketEventDatabase"})
        supported_database |= database_class == "vtable for SimBridge::" + std::string(name);
    if (!supported_database) throw Unsupported("unqualified native event database");
    reader.index_manager = symbol<void*(*)(const void*)>(
        "_ZN9SimBridge15WdbCommonReader15getBTreeManagerEv")(reader.common);
    auto root = reader.native.getRootObject();
    std::vector<const SimBridge::IObject*> objects;
    reader.native.traverseAllObjects([&](const SimBridge::IObject* object) {
        if (object != root) objects.push_back(object);
        return static_cast<SimBridge::IReader::TraverseCommand>(0);
    }, true, nullptr);
    std::vector<bool> scopes;
    scopes.reserve(objects.size());
    for (auto object : objects) {
        auto scope = vcall<int>(object, 2) == 0;
        scopes.push_back(scope);
        if (scope) ++reader.scope_count;
    }
    std::unordered_map<const SimBridge::IObject*, size_t> indices;
    indices.reserve(reader.scope_count);
    for (size_t i = 0; i < objects.size(); ++i)
        if (scopes[i]) indices.emplace(objects[i], i);
    reader.declarations.reserve(objects.size());
    auto append = [&](Declaration&& d) {
        d.id = reader.declarations.size();
        reader.declarations.push_back(std::move(d));
    };
    for (size_t i = 0; i < objects.size(); ++i) {
        Declaration d;
        d.object = objects[i]; d.id = i;
        d.name = vcall<std::string>(d.object, 4); text(d.name);
        d.name_syntax = vcall<bool>(d.object, 10) ? 1 : vcall<bool>(d.object, 9) ? 2 : 0;
        auto parent = vcall<const SimBridge::IObject*>(d.object, 6);
        if (parent && parent != root) {
            auto found = indices.find(parent);
            if (found == indices.end()) throw std::runtime_error("native parent is absent from inventory");
            d.parent = found->second;
        }
        d.scope = scopes[i];
        if (d.scope) d.kind = "scope";
        else {
            reader.meta.has_variables = 1;
            describe(reader, d);
        }
        append(std::move(d));
    }
    bool members_seen = false;
    for (size_t index = 0; index < reader.declarations.size(); ++index) {
        const auto& candidate = reader.declarations[index];
        if (candidate.scope || !candidate.qualified) continue;
        const auto& info = reader.type_info(candidate.type);
        auto type = info.native_class;
        std::string member_class;
        if (type == "vtable for SimBridge::ArrayTypeObj") {
            auto packing = info.packing;
            auto element_class = info.element_class;
            if (packing == 1 || (packing == 0 && candidate.encoding == 0 &&
                (element_class == "vtable for SimBridge::EnumTypeObj" ||
                 element_class == "vtable for SimBridge::IntTypeObj" ||
                 element_class == "vtable for SimBridge::FloatTypeObj")) || (packing == 2 &&
                (element_class == "vtable for SimBridge::DiscreteEnumArrayTypeObj" ||
                 element_class == "vtable for SimBridge::DiscreteEnumScalarTypeObj" ||
                 element_class == "vtable for SimBridge::TypedefTypeObj")))
                member_class = "ArrayDataObj";
        }
        else if (type == "vtable for SimBridge::RecordTypeObj") member_class = "RecordDataObj";
        if (member_class.empty()) continue;
        if (!members_seen) {
            // Parent lookup needs only scopes. Member deduplication also needs
            // the variables, but only when this recording expands members.
            indices.reserve(objects.size());
            for (size_t i = 0; i < objects.size(); ++i)
                if (!scopes[i]) indices.emplace(objects[i], i);
            members_seen = true;
        }
        auto parent = candidate;
        auto parent_data = pointer(parent.object, 8);
        const bool record = member_class == "RecordDataObj";
        size_t scope = parent.parent;
        if (record) {
            Declaration container;
            container.scope = true; container.name = parent.name; container.kind = parent.kind;
            container.type_name = parent.type_name; container.name_syntax = parent.name_syntax;
            container.parent = parent.parent; container.packing = parent.packing;
            scope = reader.declarations.size();
            append(std::move(container));
            ++reader.scope_count;
        }
        auto prefix = "_ZNK9SimBridge" + std::to_string(member_class.size()) + member_class;
        auto count = symbol<int(*)(const void*)>(prefix + "21getNumDataObjChildrenEv")(parent_data);
        if (count < 0 || static_cast<size_t>(count) > 1048576)
            throw Unsupported("native member count exceeds supported extent");
        auto child_at = symbol<const SimBridge::IObject*(*)(const void*, int)>(prefix + "15getDataObjChildEi");
        for (int child = 0; child < count; ++child) {
            auto object = child_at(parent_data, child);
            if (!object) throw std::runtime_error("null native member");
            if (!indices.emplace(object, reader.declarations.size()).second) continue;
            Declaration d;
            d.object = object; d.id = reader.declarations.size(); d.parent = scope;
            d.name_syntax = parent.name_syntax;
            auto name = vcall<std::string>(object, 4);
            if (record && !name.empty() && name.front() == '.') {
                d.reader_name = std::make_shared<const std::string>(name);
                name.erase(0, 1);
            }
            d.name = record ? name : parent.name + name;
            d.base_len = record ? 0 : parent.base_len ? parent.base_len : parent.name.size();
            text(d.name);
            describe(reader, d);
            append(std::move(d));
        }
    }
    const auto variables = reader.declarations.size() - reader.scope_count;
    size_t capacity = 1;
    while (capacity / 2 < variables) {
        if (capacity > std::numeric_limits<size_t>::max() / 2)
            throw Unsupported("native declaration count exceeds identity index capacity");
        capacity *= 2;
    }
    std::vector<size_t> identities(capacity, no_parent);
    for (size_t index = 0; index < reader.declarations.size(); ++index) {
        auto& d = reader.declarations[index];
        if (d.scope || d.address < 0 || !d.length || !d.encoding) continue;
        const auto key = identity(d);
        size_t hash = 0;
        std::apply([&](auto... values) {
            ((hash ^= std::hash<decltype(values)>{}(values) + 0x9e3779b9 + (hash << 6) + (hash >> 2)), ...);
        }, key);
        hash ^= hash >> 33;
        hash *= 0xff51afd7ed558ccdULL;
        hash ^= hash >> 33;
        size_t slot = hash & (capacity - 1);
        while (identities[slot] != no_parent && key != identity(reader.declarations[identities[slot]]))
            slot = (slot + 1) & (capacity - 1);
        if (identities[slot] == no_parent) identities[slot] = index;
        d.id = identities[slot];
    }

}

void error_text(char* error, size_t capacity, const char* message) {
    if (capacity) {
        auto length = std::min(capacity - 1, std::strlen(message));
        std::memcpy(error, message, length); error[length] = 0;
    }
}
template<class F> int protect(char* error, size_t capacity, F run) {
    try { return run(); }
    catch (const Unsupported& e) { error_text(error, capacity, e.what()); return -4; }
    catch (const std::exception& e) { error_text(error, capacity, e.what()); return -1; }
    catch (...) { error_text(error, capacity, "native reader exception"); return -1; }
}
}

extern "C" int ondas_wdb_open(const char* path, int metadata_only, void** out, char* error, size_t cap) {
    *out = nullptr;
    return protect(error, cap, [&] {
        auto data = getenv("RDI_DATADIR");
        std::unique_ptr<char, decltype(&free)> resolved(data ? realpath(data, nullptr) : nullptr, &free);
        if (!resolved || std::strcmp(resolved.get(), ONDAS_WDB_DATA))
            throw Unsupported("RDI_DATADIR must select the build-time reader data directory");
        selected_library("_ZN9SimBridge10DataReader13getDataObjMgrEv", ONDAS_WDB_READER);
        selected_library("_ZNK9SimBridge15WdbCommonReader13getValueEventExRKSt6vectorIPNS_14WdbScalarBTreeESaIS3_EERNS_17WdbTimeByteValuesE", ONDAS_WDB_KERNEL);
        auto reader = std::make_unique<Reader>(path);
        if (!metadata_only) import(*reader);
        else reader->native.traverseAllObjects([&](const SimBridge::IObject* object) {
            if (vcall<int>(object, 2)) reader->meta.has_variables = 1;
            return static_cast<SimBridge::IReader::TraverseCommand>(0);
        }, true, nullptr);
        *out = reader.release(); return 0;
    });
}
extern "C" void ondas_wdb_close(void* opaque) { delete static_cast<Reader*>(opaque); }
extern "C" void ondas_wdb_metadata(void* opaque, ondas_wdb_meta* out) { *out = static_cast<Reader*>(opaque)->meta; }
extern "C" size_t ondas_wdb_decl_count(void* opaque) { return static_cast<Reader*>(opaque)->declarations.size(); }
extern "C" size_t ondas_wdb_scope_count(void* opaque) { return static_cast<Reader*>(opaque)->scope_count; }
extern "C" int ondas_wdb_declaration(void* opaque, size_t index, ondas_wdb_decl* out) {
    auto& declarations = static_cast<Reader*>(opaque)->declarations;
    if (index >= declarations.size()) return -1;
    auto& d = declarations[index];
    *out = {d.id, d.parent, d.scope, d.encoding, d.width, d.has_range, d.constant,
            d.direction, 0, d.domain, d.packing, d.name_syntax, d.msb, d.lsb, d.name.c_str(),
            d.reader_name ? d.reader_name->c_str() : d.name.c_str(), d.kind,
            d.type_name, nullptr, d.variants.size(), d.base_len};
    return 0;
}
extern "C" int ondas_wdb_enum_variant(void* opaque, size_t declaration, size_t variant, const char** bits, const char** label) {
    auto& declarations = static_cast<Reader*>(opaque)->declarations;
    if (declaration >= declarations.size() || variant >= declarations[declaration].variants.size()) return -1;
    auto& value = declarations[declaration].variants[variant];
    *bits = value.bits.c_str(); *label = value.label.c_str();
    return 0;
}
extern "C" int ondas_wdb_begin(void* opaque, const uint64_t* ids, size_t count, uint64_t start,
                               uint64_t end, char* error, size_t cap) {
    auto& reader = *static_cast<Reader*>(opaque);
    reader.traversal.reset();
    return protect(error, cap, [&] {
        auto traversal = std::make_unique<Traversal>();
        traversal->end = std::min(end, reader.meta.last);
        IndexBatch batch(reader.index_manager);
        for (size_t i = 0; i < count; ++i) {
            auto& d = reader.declarations.at(ids[i]);
            if (d.scope || d.id != ids[i]) throw std::runtime_error("invalid native history identity");
            if (!d.encoding) throw Unsupported("selected native value class is not qualified");
            if (d.address < 0 || !d.length) throw Unsupported("selected declaration has no qualified native extent");
            if (d.address > std::numeric_limits<long long>::max() - d.length) throw std::runtime_error("native extent overflow");
            auto& selected = traversal->selected.emplace_back(d);
            batch.add(d.address, d.length, selected.trees);
        }
        batch.load();
        std::map<void*, size_t, std::less<void*>> indices;
        for (size_t index = 0; index < traversal->selected.size(); ++index) {
            auto& selected = traversal->selected[index];
            if (start) {
                long long tick = 0;
                reader.raw(selected, std::min(start - 1, reader.meta.last), selected.bytes, tick, &selected.known);
                selected.unknown = std::count(selected.known.begin(), selected.known.end(), 0);
                if (!selected.unknown) {
                    reader.decode(*selected.declaration, selected.bytes.data(), selected.value);
                    selected.observed = true;
                }
            }
            if (start > traversal->end) continue;
            for (size_t offset = 0; offset < selected.trees.size(); ++offset) {
                auto tree = selected.trees[offset];
                auto [entry, inserted] = indices.emplace(tree, traversal->cursors.size());
                if (inserted) {
                    auto cursor = std::make_unique<ByteCursor>(tree, start, reader.meta.last, traversal->end);
                    if (cursor->valid) traversal->queue.emplace(cursor->current.tick, traversal->cursors.size());
                    traversal->cursors.push_back(std::move(cursor));
                }
                traversal->cursors[entry->second]->targets.emplace_back(index, offset);
            }
        }
        reader.traversal = std::move(traversal);
        return 0;
    });
}
extern "C" int ondas_wdb_sample_bits(void* opaque, uint64_t id, uint64_t time, uint32_t lsb,
                                     uint32_t width, ondas_wdb_record* out, int* changed,
                                     char* error, size_t cap) {
    auto& reader = *static_cast<Reader*>(opaque);
    *out = {}; *changed = 0;
    return protect(error, cap, [&] {
        if (!reader.traversal) throw std::runtime_error("native traversal is not active");
        auto& t = *reader.traversal;
        auto found = std::find_if(t.selected.begin(), t.selected.end(), [&](const auto& s) {
            return s.declaration->id == id;
        });
        if (found == t.selected.end()) throw std::runtime_error("point signal is not selected");
        const auto& selected = *found;
        const auto& d = *selected.declaration;
        if (d.encoding != 1 || !width || uint64_t(lsb) + width > d.width)
            throw std::runtime_error("invalid native point bit projection");
        std::vector<unsigned char> raw, decoded, value;
        long long at = 0;
        if (!reader.raw(selected, std::min(time, reader.meta.last), raw, at)) return 0;
        reader.decode(d, raw.data(), decoded);
        const auto first = d.width - lsb - width;
        value.assign(decoded.begin() + first, decoded.begin() + first + width);
        while (at > 0) {
            long long before = 0;
            if (!reader.raw(selected, at - 1, raw, before)) break;
            if (before >= at) throw std::runtime_error("native point predecessor did not move backwards");
            reader.decode(d, raw.data(), decoded);
            if (!std::equal(value.begin(), value.end(), decoded.begin() + first)) {
                *changed = 1;
                break;
            }
            at = before;
        }
        t.output = std::move(value);
        *out = {static_cast<uint64_t>(at), id, 1, t.output.data(), t.output.size()};
        return 1;
    });
}
extern "C" int ondas_wdb_next(void* opaque, ondas_wdb_record* out, char* error, size_t cap) {
    auto& reader = *static_cast<Reader*>(opaque);
    return protect(error, cap, [&] {
        if (!reader.traversal) throw std::runtime_error("native traversal is not active");
        auto& t = *reader.traversal;
        for (;;) {
            if (!t.pending) {
                if (t.queue.empty() || static_cast<uint64_t>(t.queue.top().first) > t.end) return 0;
                t.tick = t.queue.top().first;
                // Finish all bytes at this tick before decoding an aggregate.
                do {
                    auto item = t.queue.top(); t.queue.pop();
                    auto& cursor = *t.cursors[item.second];
                    if (cursor.current.byte.kind != 1)
                        throw std::runtime_error("native byte iterator returned a non-value record");
                    for (auto [index, offset] : cursor.targets) {
                        auto& selected = t.selected[index];
                        auto byte = cursor.current.byte.value;
                        selected.dirty |= !selected.known[offset] || selected.bytes[offset] != byte;
                        selected.bytes[offset] = byte;
                        if (!selected.known[offset]) { selected.known[offset] = 1; --selected.unknown; }
                    }
                    cursor.advance();
                    if (cursor.valid) t.queue.emplace(cursor.current.tick, item.second);
                } while (!t.queue.empty() && t.queue.top().first == t.tick);
                t.selection = 0;
                t.pending = true;
            }
            while (t.selection < t.selected.size()) {
                auto& selected = t.selected[t.selection++];
                if (!selected.dirty) continue;
                selected.dirty = false;
                if (selected.unknown) continue;
                auto& d = *selected.declaration;
                reader.decode(d, selected.bytes.data(), t.output);
                if (selected.observed && selected.value == t.output) continue;
                selected.value.swap(t.output);
                selected.observed = true;
                *out = {static_cast<uint64_t>(t.tick), d.id, d.encoding, selected.value.data(), selected.value.size()};
                return 1;
            }
            t.pending = false;
        }
    });
}
extern "C" void ondas_wdb_end(void* opaque) { static_cast<Reader*>(opaque)->traversal.reset(); }
