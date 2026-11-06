// Exercise the actual adapter and Selected containers through a controlled ABI.
// This does not qualify the installed reader or replace recorded-input checks.
#include <dlfcn.h>
static void* test_symbol(void*, const char*);
#define dlsym test_symbol
#include "../../native/wdb.cpp"
#undef dlsym
#include <cassert>

namespace {
struct Event { long long tick; unsigned char value; };
struct Tree { long long address; unsigned bit; bool loaded = false; std::vector<Event> events; };
struct Manager {
    std::map<std::pair<long long, unsigned>, std::unique_ptr<Tree>> trees;
    size_t scans = 0, finds = 0, drops = 0, removals = 0;
    bool fail_scan = false, bad_raw = false, missing_delete = false;
};
Manager* active = nullptr;
struct Iterator { void** vtable; Tree* tree; size_t position; long long end; };
std::vector<Event> events(long long address) {
    if (address == 100000) return {};
    if (address == 100001) return {{4, 3}, {9, 7}};
    if (address == 100002) return {{0, 42}};
    auto byte = static_cast<unsigned char>(address);
    return {{0, byte}, {4, static_cast<unsigned char>(byte ^ 1)},
            {9, static_cast<unsigned char>(byte ^ 3)}};
}
void add(Manager& m, long long address, unsigned bit, std::vector<void*>& trees, std::vector<void*>& created) {
    auto [it, inserted] = m.trees.try_emplace({address, bit});
    if (inserted) {
        it->second = std::make_unique<Tree>(Tree{address, bit, false, events(address)});
        created.push_back(it->second.get());
    }
    trees.push_back(it->second.get());
}
void create_bytes(void* opaque, long long address, long long length, std::vector<void*>& trees, std::vector<void*>& created) {
    auto& m = *static_cast<Manager*>(opaque);
    for (long long i = 0; i < length; ++i) add(m, address + i, 0, trees, created);
}
void create_bits(void* opaque, const std::vector<AddressBitPair>& pairs, std::vector<void*>& trees, std::vector<void*>& created) {
    auto& m = *static_cast<Manager*>(opaque);
    for (auto p : pairs) add(m, p.address, p.bit, trees, created);
}
long long tree_address(const void* tree) { return static_cast<const Tree*>(tree)->address; }
unsigned tree_bit(const void* tree) { return static_cast<const Tree*>(tree)->bit; }
void remove_bytes(void* opaque, long long address, long long length) {
    auto& m = *static_cast<Manager*>(opaque);
    for (long long i = 0; i < length; ++i) m.removals += m.trees.erase({address + i, 0});
}
void remove_bits(void* opaque, const std::vector<AddressBitPair>& pairs) {
    auto& m = *static_cast<Manager*>(opaque);
    for (auto p : pairs) m.removals += m.trees.erase({p.address, p.bit});
}
void scan(void* opaque, const std::vector<void*>& created, void*) {
    auto& m = *static_cast<Manager*>(opaque); ++m.scans;
    if (m.fail_scan) throw std::runtime_error("controlled load failure");
    for (auto p : created) static_cast<Tree*>(p)->loaded = true;
}
unsigned char stored(const Tree& tree, unsigned char byte) {
    return ONDAS_WDB_BIT_INDEX ? (byte >> tree.bit) & 1 : byte;
}
void raw(const void*, long long time, const std::vector<void*>& trees, TimeByteValues& out) {
    if (active->bad_raw) throw std::runtime_error("controlled raw failure");
    for (auto p : trees) {
        auto& tree = *static_cast<Tree*>(p); assert(tree.loaded);
        ByteValue byte{};
        for (auto e : tree.events) if (e.tick <= time) {
            byte.kind = 1; byte.value = stored(tree, e.value); out.tick = std::max<int64_t>(out.tick, e.tick);
        }
        out.bytes.push_back(byte);
    }
}
void destroy(const void* pointer) { ++active->drops; delete static_cast<const Iterator*>(pointer); }
void* find(const void* pointer, long long start, long long, long long, long long end) {
    ++active->finds;
    auto* tree = const_cast<Tree*>(static_cast<const Tree*>(pointer)); assert(tree->loaded);
    static void* vtable[] = {nullptr, reinterpret_cast<void*>(destroy)};
    size_t at = 0;
    while (at < tree->events.size() && tree->events[at].tick < start) ++at;
    return new Iterator{vtable, tree, at, end};
}
bool present(const void* pointer) {
    auto& p = *static_cast<const Iterator*>(pointer);
    return p.position < p.tree->events.size() && p.tree->events[p.position].tick <= p.end;
}
TimeByteValue current(const void* pointer) {
    auto& p = *static_cast<const Iterator*>(pointer); auto e = p.tree->events.at(p.position);
    return {e.tick, {1, stored(*p.tree, e.value), {}}};
}
void* advance(void* pointer) { ++static_cast<Iterator*>(pointer)->position; return pointer; }

struct Observation { int status, changed; uint64_t tick; std::string value; };
Observation point(Reader& reader, uint64_t id, uint64_t tick, unsigned lsb = 0, unsigned width = 8) {
    ondas_wdb_record row{}; int changed = 0; char error[256]{};
    auto status = ondas_wdb_sample_bits(&reader, id, tick, lsb, width, &row, &changed, error, sizeof(error));
    return {status, changed, row.tick, status > 0 ? std::string(row.data, row.data + row.len) : std::string{}};
}
void equal(const Observation& a, const Observation& b) {
    assert(a.status == b.status && a.changed == b.changed && a.tick == b.tick && a.value == b.value);
}
void check_selection(Reader& reader, const std::vector<uint64_t>& ids) {
    char error[256]{}; auto finds = active->finds;
    assert(ondas_wdb_begin_points(&reader, ids.data(), ids.size(), error, sizeof(error)) == 0);
    auto& t = *reader.traversal; assert(t.cursors.empty() && t.queue.empty() && active->finds == finds);
    assert(t.selected.size() == ids.size());
    for (auto id : ids) {
        auto first = std::find_if(t.selected.begin(), t.selected.end(), [&](const auto& selected) { return selected.declaration->id == id; });
        assert(first != t.selected.end());
        assert(t.point_positions.at(id) == static_cast<size_t>(first - t.selected.begin()));
        auto expected = point(reader, id, 9);
        assert(expected.status == 1);
        assert(expected.value.size() == 8);
    }
    assert(point(reader, 10006, 0).status == -1);
    ondas_wdb_end(&reader); assert(!reader.traversal);
}
}

static void* test_symbol(void*, const char* name) {
    const std::string s(name);
#define MATCH(part, function) if (s.find(part) != std::string::npos) return reinterpret_cast<void*>(function)
    MATCH("getOrCreateByteBTrees", create_bytes); MATCH("getOrCreateBitBTrees", create_bits);
    MATCH("getAddressEv", tree_address); MATCH("getBitPositionEv", tree_bit);
    if (!active->missing_delete) {
        MATCH("deleteByteBTrees", remove_bytes); MATCH("deleteBitBTrees", remove_bits);
    }
    MATCH("scanWdbDatabase", scan); MATCH("getValueEvent", raw);
    MATCH("findExxxx", find); MATCH("IteratorcvbEv", present);
    MATCH("IteratordeEv", current); MATCH("IteratorppEv", advance);
#undef MATCH
    return nullptr;
}
namespace SimBridge {
DataReader::DataReader(WdbOpenMode, bool, const std::string&, const std::string&, const std::vector<std::string>&, int) {}
DataReader::~DataReader() = default;
int DataReader::getPrecision() const { return -9; }
long long DataReader::getLatestTime() const { return 100; }
void DataConversion::formatRawMemory(const BitIndex<const unsigned char*>& bytes, const IType*, const RTTI::FullConstraints*, const ValueFormat&, bool, Value& value) const {
    value.kind = 7; value.payload.text = new char[9];
    for (unsigned i = 0; i < 8; ++i) value.payload.text[i] = '0' + ((*bytes.bytes >> (7 - i)) & 1);
    value.payload.text[8] = 0;
}
}

int main(int argc, char** argv) {
    Manager manager; active = &manager; Reader reader("controlled"); SimBridge::DataConversion conversion;
    reader.index_manager = &manager; reader.common = &manager; reader.conversion = &conversion;
    // Early fault mode: the deletion ABI is unavailable for this process, so the first index
    // preparation must fail before any native tree exists. Nothing is created and no rollback
    // runs while an exception unwinds.
    if (argc > 1 && std::string(argv[1]) == "missing-delete") {
        manager.missing_delete = true;
        reader.declarations.resize(10007);
        for (size_t i = 0; i < reader.declarations.size(); ++i) {
            auto& d = reader.declarations[i]; d.id = i; d.address = i; d.length = 1; d.width = 8; d.encoding = 1;
        }
        const uint64_t id = 31; char fault_error[256]{};
        assert(ondas_wdb_begin_points(&reader, &id, 1, fault_error, sizeof(fault_error)) == -4);
        assert(!reader.traversal);
        assert(reader.histories.empty());
        assert(manager.trees.empty());
        assert(manager.removals == 0);
        return 0;
    }
    reader.declarations.resize(10007);
    for (size_t i = 0; i < reader.declarations.size(); ++i) {
        auto& d = reader.declarations[i]; d.id = i; d.address = i; d.length = 1; d.width = 8; d.encoding = 1;
    }
    check_selection(reader, {}); check_selection(reader, {31}); check_selection(reader, {173, 1, 31, 173, 1});
    std::vector<uint64_t> large;
    for (uint64_t i = 5000; i > 0; --i) large.push_back(i * 2);
    large.push_back(2); large.push_back(10000); check_selection(reader, large);

    reader.declarations[3].address = 100000; // no recorded history
    reader.declarations[4].address = 100001; // first recorded at tick4
    reader.declarations[5].address = 100002; // constant
    reader.declarations[6].address = reader.declarations[4].address; // shared extent, distinct ID
    const std::vector<uint64_t> ids{5, 4, 3, 6, 4, 1}; char error[256]{};
    for (uint64_t tick : std::vector<uint64_t>{0, 3, 4, 8, 9, 100, UINT64_MAX}) {
        // Range preparation is the independent, unchanged selection/reference path.
        assert(ondas_wdb_begin(&reader, ids.data(), ids.size(), 0, 100, error, sizeof(error)) == 0);
        std::vector<Observation> original;
        for (auto id : ids) original.push_back(point(reader, id, tick));
        assert(ondas_wdb_begin_points(&reader, ids.data(), ids.size(), error, sizeof(error)) == 0);
        for (size_t i = 0; i < ids.size(); ++i) equal(point(reader, ids[i], tick), original[i]);
        assert(point(reader, 3, tick).status == 0);
        assert(point(reader, 4, tick).status == (tick < 4 ? 0 : 1));
        assert(point(reader, 5, tick).changed == 0 && point(reader, 5, tick).tick == 0);
        assert(point(reader, 4, tick, 8, 1).status == -1);
        assert(point(reader, 9999, tick).status == -1);
        assert(point(reader, 1, tick).status == 1);
        active->bad_raw = true; assert(point(reader, 1, tick).status == -1); active->bad_raw = false;
        assert(point(reader, 1, tick).status == 1);
        ondas_wdb_end(&reader);
    }
    assert(manager.finds == manager.drops);
    assert(ondas_wdb_begin(&reader, ids.data(), ids.size(), 0, 100, error, sizeof(error)) == 0);
    ondas_wdb_record row{}; assert(ondas_wdb_next(&reader, &row, error, sizeof(error)) == 1);
    const uint64_t invalid = UINT64_MAX;
    assert(ondas_wdb_begin_points(&reader, &invalid, 1, error, sizeof(error)) == -1 && !reader.traversal);
    assert(manager.finds == manager.drops);
    const uint64_t fresh = 10004; manager.fail_scan = true; auto removed = manager.removals;
    assert(ondas_wdb_begin_points(&reader, &fresh, 1, error, sizeof(error)) == -1 && !reader.traversal);
    assert(manager.removals == removed + indexes_per_byte);
    manager.fail_scan = false;
    assert(ondas_wdb_begin_points(&reader, &fresh, 1, error, sizeof(error)) == 0);
    auto scans = manager.scans;
    assert(point(reader, fresh, UINT64_MAX).status == 1);
    assert(ondas_wdb_begin_points(&reader, &fresh, 1, error, sizeof(error)) == 0 && manager.scans == scans);
    ondas_wdb_end(&reader); ondas_wdb_end(&reader);
    assert(point(reader, fresh, 0).status == -1);

    // Owner-level index accounting: park deletes every adapter-created SDK index exactly once,
    // clears the traversal first, and a second release cannot delete anything again.
    assert(ondas_wdb_begin_points(&reader, ids.data(), ids.size(), error, sizeof(error)) == 0);
    assert(ondas_wdb_begin(&reader, &fresh, 1, 0, 100, error, sizeof(error)) == 0);
    ondas_wdb_end(&reader);
    auto owned = reader.histories.size();
    assert(owned > 0);
    auto before_release = manager.removals;
    assert(ondas_wdb_release_indexes(&reader, error, sizeof(error)) == 0);
    assert(!reader.traversal);
    assert(reader.histories.empty());
    assert(manager.removals == before_release + owned);
    auto after_release = manager.removals;
    assert(ondas_wdb_release_indexes(&reader, error, sizeof(error)) == 0);
    assert(manager.removals == after_release);
    // A released owner must be able to create and load the same indexes again, and an active
    // traversal must be dropped before its indexes are deleted.
    assert(ondas_wdb_begin_points(&reader, ids.data(), ids.size(), error, sizeof(error)) == 0);
    assert(point(reader, 1, 9).status == 1);
    auto recreated = reader.histories.size();
    assert(recreated > 0);
    before_release = manager.removals;
    assert(ondas_wdb_release_indexes(&reader, error, sizeof(error)) == 0);
    assert(!reader.traversal && reader.histories.empty());
    assert(manager.removals == before_release + recreated);

}
