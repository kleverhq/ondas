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
    size_t fail_remove_after = std::numeric_limits<size_t>::max();
    bool fail_scan = false, bad_raw = false, missing_delete = false;
};
Manager* active = nullptr;
struct Iterator { void** vtable; Tree* tree; size_t position; long long end; };
std::vector<Event> events(long long address) {
    if (address == 100000) return {};
    if (address == 100001) return {{4, 3}, {9, 7}};
    if (address == 100002) return {{0, 42}};
    // Controlled two-chunk two-plane extent for the origin-fallback regression:
    // chunk 0 = 100010..100017 (aval 100010..100013, bval 100014..100017), logical bits 0..31;
    // chunk 1 = 100018..100025 (aval 100018..100021, bval 100022..100025), logical bits 32..63.
    // Logical bit 0 is aval byte 0 bit 0 with bval byte 4 bit 0, so a projection of that bit
    // covers chunk 0 only. Byte 100025 (bval bit 63) is first recorded at tick 5, i.e. outside
    // the covered chunk, so the extent is incomplete before tick 5 while bit 0 already changed
    // at tick 3. Expecting that difference is what makes the whole-extent origin guard matter.
    if (address == 100010) return {{0, 0x01}, {3, 0x00}, {12, 0x01}};
    if (address >= 100011 && address <= 100017) return {{0, 0x00}};
    if (address == 100018) return {{0, 0x00}, {16, 0x01}};
    if (address >= 100019 && address <= 100024) return {{0, 0x00}};
    if (address == 100025) return {{5, 0x00}, {16, 0x80}};
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
    if (m.removals >= m.fail_remove_after) throw std::runtime_error("controlled delete failure");
    for (long long i = 0; i < length; ++i) m.removals += m.trees.erase({address + i, 0});
}
void remove_bits(void* opaque, const std::vector<AddressBitPair>& pairs) {
    auto& m = *static_cast<Manager*>(opaque);
    if (m.removals >= m.fail_remove_after) throw std::runtime_error("controlled delete failure");
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

    // Session read-set pruning: a clean park keeps only the histories covered by the
    // declarations the last index-using session selected; a session without a selection keeps
    // the previous cache; reused and alias/overlap trees stay in the read set by extent.
    {
        const std::vector<uint64_t> first{1, 4, 10006};
        assert(ondas_wdb_begin_points(&reader, first.data(), first.size(), error, sizeof(error)) == 0);
        ondas_wdb_end(&reader);
        const auto first_owned = reader.histories.size();
        assert(first_owned == first.size() * indexes_per_byte);
        auto before = manager.removals;
        assert(ondas_wdb_prune_indexes(&reader, error, sizeof(error)) == 0);
        assert(manager.removals == before && reader.histories.size() == first_owned);
        const std::vector<uint64_t> second{5};
        assert(ondas_wdb_begin_points(&reader, second.data(), second.size(), error, sizeof(error)) == 0);
        ondas_wdb_end(&reader);
        assert(reader.histories.size() == first_owned + second.size() * indexes_per_byte);
        assert(ondas_wdb_prune_indexes(&reader, error, sizeof(error)) == 0);
        assert(reader.histories.size() == second.size() * indexes_per_byte);
        assert(manager.removals == before + first_owned);
        before = manager.removals;
        assert(ondas_wdb_begin_points(&reader, second.data(), second.size(), error, sizeof(error)) == 0);
        ondas_wdb_end(&reader);
        assert(ondas_wdb_prune_indexes(&reader, error, sizeof(error)) == 0);
        assert(reader.histories.size() == second.size() * indexes_per_byte);
        assert(manager.removals == before);
        const std::vector<uint64_t> aliases{4, 6};
        assert(ondas_wdb_begin_points(&reader, aliases.data(), aliases.size(), error, sizeof(error)) == 0);
        ondas_wdb_end(&reader);
        assert(ondas_wdb_prune_indexes(&reader, error, sizeof(error)) == 0);
        const auto alias_owned = reader.histories.size();
        assert(alias_owned == indexes_per_byte);
        const std::vector<uint64_t> shared{6};
        assert(ondas_wdb_begin_points(&reader, shared.data(), shared.size(), error, sizeof(error)) == 0);
        ondas_wdb_end(&reader);
        assert(ondas_wdb_prune_indexes(&reader, error, sizeof(error)) == 0);
        assert(reader.histories.size() == alias_owned);
        auto& overlapping = reader.declarations[7];
        overlapping.address = 100001;
        overlapping.length = 2;
        overlapping.width = 16;
        overlapping.encoding = 1;
        const std::vector<uint64_t> overlap{7};
        assert(ondas_wdb_begin_points(&reader, overlap.data(), overlap.size(), error, sizeof(error)) == 0);
        ondas_wdb_end(&reader);
        assert(ondas_wdb_prune_indexes(&reader, error, sizeof(error)) == 0);
        assert(reader.histories.size() == 2 * indexes_per_byte);
        // Partial prune failure: the failing tree is erased before its cleanup throws, so a
        // later full release deletes the remaining trees and never the erased one twice.
        const std::vector<uint64_t> absent{3};
        assert(ondas_wdb_begin_points(&reader, absent.data(), absent.size(), error, sizeof(error)) == 0);
        ondas_wdb_end(&reader);
        before = manager.removals;
        manager.fail_remove_after = before;
        assert(ondas_wdb_prune_indexes(&reader, error, sizeof(error)) == -1);
        const auto partial = reader.histories.size();
        assert(partial > indexes_per_byte && partial < 3 * indexes_per_byte);
        manager.fail_remove_after = std::numeric_limits<size_t>::max();
        assert(ondas_wdb_release_indexes(&reader, error, sizeof(error)) == 0);
        assert(reader.histories.empty());
        assert(manager.removals == before + partial);
        before = manager.removals;
        assert(ondas_wdb_prune_indexes(&reader, error, sizeof(error)) == 0);
        assert(manager.removals == before);
    }

    // Origin fallback on the qualified two-plane layout, driven by an explicit history:
    //   two chunks of 32 logical bits (8 stored bytes each), extent 100010..100025;
    //   logical bit 0 lives in chunk 0 and is 1 at tick 0, 0 at tick 3, 1 at tick 12;
    //   byte 100025 (bval bit 63, in chunk 1) is first recorded at tick 5 and set at tick 16;
    //   byte 100018 (aval bit 32, also in chunk 1) is set at tick 16.
    // A projection of bit 0 covers chunk 0 only, so the late byte and both unrelated changes are
    // outside the compared bits. Before tick 5 the extent is incomplete, so a point read is
    // Missing even though bit 0 already changed at tick 3, and the first complete state must not
    // report that earlier change. A later real change at tick 12 and the unrelated ticks at 16
    // then select 12 as the run start of the projected bit, while the full-width projection
    // changes at 16. Expectations come from this history, not from the mask construction.
    // The qualified two-plane decoder is compiled for the byte-index reader only, so the older
    // reader keeps its previous path and this regression does not apply to it.
    if constexpr (!ONDAS_WDB_BIT_INDEX) {
        auto& d = reader.declarations[100];
        d.address = 100010; d.length = 16; d.width = 64; d.encoding = 1; d.two_plane = true;
        const uint64_t projected = 100;
        assert(ondas_wdb_begin_points(&reader, &projected, 1, error, sizeof(error)) == 0);
        for (uint64_t tick : {0, 1, 2, 3, 4}) assert(point(reader, projected, tick, 0, 1).status == 0);
        for (uint64_t tick : {5, 7, 11}) {
            auto slice = point(reader, projected, tick, 0, 1);
            assert(slice.status == 1 && slice.changed == 0 && slice.tick == 5);
            assert(slice.value == "0");
        }
        for (uint64_t tick : {12, 14, 15}) {
            auto slice = point(reader, projected, tick, 0, 1);
            assert(slice.status == 1 && slice.changed == 1 && slice.tick == 12);
            assert(slice.value == "1");
        }
        for (uint64_t tick : {16, 18, 20}) {
            auto slice = point(reader, projected, tick, 0, 1);
            assert(slice.status == 1 && slice.changed == 1 && slice.tick == 12);
            assert(slice.value == "1");
        }
        auto whole = point(reader, projected, 20, 0, 64);
        assert(whole.status == 1 && whole.changed == 1 && whole.tick == 16);
        assert(whole.value.size() == 64 && whole.value.front() == 'z' && whole.value.back() == '1');
        ondas_wdb_end(&reader);
        assert(!reader.traversal);
    }
}
