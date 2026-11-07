// Exercise the actual adapter and Selected containers through a controlled ABI.
// This does not qualify the installed reader or replace recorded-input checks.
#include <dlfcn.h>
static void* test_symbol(void*, const char*);
#define dlsym test_symbol
#include "../../native/wdb.cpp"
#undef dlsym
#include <cassert>
#include <mutex>
#include <set>
#include <thread>

namespace SimBridge {
struct WdbIEventBlockIterator {
    virtual ~WdbIEventBlockIterator() = default;
};
struct WdbMultipleBucketEventDatabase {
    virtual ~WdbMultipleBucketEventDatabase() = default;
};
struct WdbMultipleBucketBtreeDatabase {
    virtual ~WdbMultipleBucketBtreeDatabase() = default;
};
}

namespace {
struct Event { long long tick; unsigned char value; };
struct Tree { long long address; unsigned bit; bool loaded = false; std::vector<Event> events; };
struct Manager {
    std::map<std::pair<long long, unsigned>, std::unique_ptr<Tree>> trees;
    size_t scans = 0, finds = 0, drops = 0, removals = 0;
    size_t fail_remove_after = std::numeric_limits<size_t>::max();
    bool fail_scan = false, bad_raw = false, missing_delete = false;
    size_t parallel_groups = 4, parallel_tasks = 0, seam_hits = 0;
    bool missing_parallel = false, fail_task = false, omit_empty = false;
    std::mutex parallel_mutex;
    std::set<std::thread::id> parallel_threads;
};
Manager* active = nullptr;
#ifdef ONDAS_WDB_TEST_THREAD_LIMIT
extern "C" void ondas_wdb_test_worker_start(size_t index) {
    assert(index == ONDAS_WDB_TEST_THREAD_LIMIT);
    ++active->seam_hits;
}
#endif
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

SimBridge::WdbMultipleBucketEventDatabase event_database;
SimBridge::WdbMultipleBucketBtreeDatabase btree_database;
struct DbCommon { void* unused; const void* database; };
DbCommon db_common{nullptr, &event_database};
alignas(8) unsigned char event_header[0x48]{};
unsigned event_capacity = 64;

constexpr const char* splitter_name =
    "_ZNK9SimBridge30WdbMultipleBucketEventDatabase29getEventBlockDatabaseIterator"
    "ERKSt6vectorIPNS_14WdbScalarBTreeESaIS3_EERS1_ISt10shared_ptrINS_22WdbIEventBlockIteratorEESaISA_EERS1_IS5_SaIS5_EE";
constexpr const char* header_name =
    "_ZNK9SimBridge30WdbMultipleBucketEventDatabase22getEventDatabaseHeaderEv";
constexpr const char* task_name =
    "_ZN9SimBridge26ScanEventBlockTaskImprovedESt10shared_ptrINS_22WdbIEventBlockIteratorEE"
    "St6vectorIPNS_14WdbScalarBTreeESaIS5_EEj";

void event_splitter(const void*, const std::vector<void*>& trees,
                    std::vector<std::shared_ptr<SimBridge::WdbIEventBlockIterator>>& iterators,
                    std::vector<std::vector<void*>>& groups) {
    auto& m = *active;
    const size_t count = std::max<size_t>(1, m.parallel_groups);
    iterators.clear();
    groups.assign(count, {});
    for (size_t i = 0; i < count; ++i)
        iterators.push_back(std::make_shared<SimBridge::WdbIEventBlockIterator>());
    for (size_t i = 0; i < trees.size(); ++i) {
        if (m.omit_empty && static_cast<Tree*>(trees[i])->events.empty()) continue;
        groups[i % count].push_back(trees[i]);
    }
}

const void* event_header_fn(const void*) {
    *reinterpret_cast<uint32_t*>(event_header + 0x40) = event_capacity;
    return event_header;
}

void event_task(std::shared_ptr<SimBridge::WdbIEventBlockIterator>, std::vector<void*> group, unsigned) {
    auto& m = *active;
    bool fail_this = false;
    {
        std::lock_guard<std::mutex> lock(m.parallel_mutex);
        m.parallel_threads.insert(std::this_thread::get_id());
        ++m.parallel_tasks;
        fail_this = m.fail_task && m.parallel_tasks == 1;
    }
    if (fail_this) {
        for (auto tree : group) static_cast<Tree*>(tree)->loaded = true;
        throw std::runtime_error("controlled event task failure");
    }
    for (auto tree : group) static_cast<Tree*>(tree)->loaded = true;
}
unsigned char stored(const Tree& tree, unsigned char byte) {
    return ONDAS_WDB_BIT_INDEX ? (byte >> tree.bit) & 1 : byte;
}
void raw(const void*, long long time, const std::vector<void*>& trees, TimeByteValues& out) {
    if (active->bad_raw) throw std::runtime_error("controlled raw failure");
    for (auto p : trees) {
        auto& tree = *static_cast<Tree*>(p);
        // An omitted unlogged tree is never scanned; the adapter must read it as Missing.
        if (!tree.loaded) {
            if (active->omit_empty) { out.bytes.push_back(ByteValue{}); continue; }
            assert(tree.loaded);
        }
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

void init_declarations(Reader& reader) {
    reader.declarations.resize(20000);
    for (size_t i = 0; i < reader.declarations.size(); ++i) {
        auto& d = reader.declarations[i];
        d.id = i; d.address = i; d.length = 1; d.width = 8; d.encoding = 1;
    }
}

std::vector<uint64_t> ids_from(uint64_t base, size_t count) {
    std::vector<uint64_t> ids;
    ids.reserve(count);
    for (uint64_t i = 0; i < count; ++i) ids.push_back(base + i);
    return ids;
}

std::vector<Observation> observe(Reader& reader, const std::vector<uint64_t>& ids,
                                 const std::vector<uint64_t>& ticks) {
    std::vector<Observation> observations;
    for (auto tick : ticks)
        for (auto id : ids) observations.push_back(point(reader, id, tick));
    return observations;
}

void release_all(Reader& reader, char* error) {
    ondas_wdb_end(&reader);
    assert(ondas_wdb_release_indexes(&reader, error, 256) == 0);
}

void assert_observations_equal(const std::vector<Observation>& expected,
                               const std::vector<Observation>& actual, const char* where) {
    assert(expected.size() == actual.size());
    for (size_t i = 0; i < expected.size(); ++i)
        if (expected[i].status != actual[i].status || expected[i].changed != actual[i].changed ||
            expected[i].tick != actual[i].tick || expected[i].value != actual[i].value) {
            std::fprintf(stdout, "FAIL observation %s index=%zu\n", where, i);
            std::abort();
        }
}

std::vector<Observation> serial_baseline(Reader& reader, const std::vector<uint64_t>& ids,
                                         const std::vector<uint64_t>& probe,
                                         const std::vector<uint64_t>& ticks, char* error) {
    reader.common = &db_common;
    db_common.database = &btree_database;  // known class: symbols resolve, path is serial
    active->parallel_tasks = 0;
    active->scans = 0;
    assert(ondas_wdb_begin_points(&reader, ids.data(), ids.size(), error, 256) == 0);
    auto baseline = observe(reader, probe, ticks);
    release_all(reader, error);
    assert(active->parallel_tasks == 0 && active->scans == 1);
    db_common.database = &event_database;
    return baseline;
}

void run_missing_capability(Reader& reader) {
    char error[256]{};
    reader.common = &db_common;
    db_common.database = &event_database;
    active->missing_parallel = true;
    active->parallel_tasks = 0;
    active->scans = 0;
    const auto many = ids_from(12000, 5000);
    assert(ondas_wdb_begin_points(&reader, many.data(), many.size(), error, sizeof error) == 0);
    assert(active->parallel_tasks == 0 && active->scans == 1);
    release_all(reader, error);
}

void run_parallel_suite(Reader& reader) {
    char error[256]{};
    const auto many = ids_from(12000, 5000);
    const auto probe = ids_from(12000, 4);
    const std::vector<uint64_t> ticks{0, 9, UINT64_MAX};
    const auto baseline = serial_baseline(reader, many, probe, ticks, error);

    const auto few = ids_from(12000, 1000);
    active->parallel_tasks = 0; active->scans = 0;
    assert(ondas_wdb_begin_points(&reader, few.data(), few.size(), error, sizeof error) == 0);
    release_all(reader, error);
    assert(active->parallel_tasks == 0 && active->scans == 1);

    active->parallel_tasks = 0; active->scans = 0; active->parallel_threads.clear();
    assert(ondas_wdb_begin_points(&reader, many.data(), many.size(), error, sizeof error) == 0);
    const auto parallel = observe(reader, probe, ticks);
    assert_observations_equal(baseline, parallel, "parallel");
    if constexpr (ONDAS_WDB_BIT_INDEX) {
        assert(active->parallel_tasks == 0 && active->scans == 1);
    } else if (available_workers() <= 1) {
        assert(active->parallel_tasks == 0 && active->scans == 1);
    } else {
        assert(active->scans == 0);
        assert(active->parallel_tasks == active->parallel_groups);
        assert(active->parallel_threads.size() >= 2);
        assert(reader.histories.size() == many.size() * indexes_per_byte);
    }
    release_all(reader, error);
}

void run_omission(Reader& reader) {
    char error[256]{};
    reader.declarations[13000].address = 100000;  // recorded presence: no events
    const auto many = ids_from(12000, 5000);
    const auto probe = ids_from(12000, 4);
    const std::vector<uint64_t> ticks{0, 9, UINT64_MAX};
    const auto baseline = serial_baseline(reader, many, probe, ticks, error);

    active->omit_empty = true;
    active->parallel_tasks = 0; active->scans = 0; active->parallel_threads.clear();
    assert(ondas_wdb_begin_points(&reader, many.data(), many.size(), error, sizeof error) == 0);
    const auto parallel = observe(reader, probe, ticks);
    assert_observations_equal(baseline, parallel, "omission");
    assert(point(reader, 13000, 0).status == 0);
    if constexpr (ONDAS_WDB_BIT_INDEX) {
        assert(active->parallel_tasks == 0 && active->scans == 1);
    } else if (available_workers() > 1) {
        assert(active->parallel_tasks == active->parallel_groups && active->scans == 0);
    }
    release_all(reader, error);
    active->omit_empty = false;
}

void run_start_failure(Reader& reader) {
    char error[256]{};
    active->parallel_groups = 8;
    const auto many = ids_from(12000, 5000);
    const auto probe = ids_from(12000, 4);
    const std::vector<uint64_t> ticks{0, 9, UINT64_MAX};
    const auto baseline = serial_baseline(reader, many, probe, ticks, error);

    active->parallel_tasks = 0; active->scans = 0; active->seam_hits = 0;
    assert(ondas_wdb_begin_points(&reader, many.data(), many.size(), error, sizeof error) == 0);
    const auto fallback = observe(reader, probe, ticks);
    assert_observations_equal(baseline, fallback, "start-failure");
    if (available_workers() >= 2) {
        // A real worker was created before the controlled creation failure; the gate aborted,
        // joined it and fell back to serial without running a task.
        assert(active->seam_hits == 1);
        assert(active->parallel_tasks == 0 && active->scans == 1);
    } else {
        // With a 0/1 budget the product correctly starts no worker and stays serial.
        assert(active->seam_hits == 0);
        assert(active->parallel_tasks == 0 && active->scans == 1);
    }
    active->parallel_groups = 4;
    release_all(reader, error);
}

void run_task_failure(Reader& reader) {
    char error[256]{};
    const auto many = ids_from(12000, 5000);
    const auto probe = ids_from(12000, 4);
    const std::vector<uint64_t> ticks{0, 9, UINT64_MAX};
    const auto baseline = serial_baseline(reader, many, probe, ticks, error);

    active->fail_task = true;
    active->parallel_tasks = 0; active->scans = 0;
    const int failed = ondas_wdb_begin_points(&reader, many.data(), many.size(), error, sizeof error);
    if (available_workers() < 2) {
        // No eligible worker: the product stays serial and the fake task is never called.
        assert(failed == 0);
        const auto serial = observe(reader, probe, ticks);
        assert_observations_equal(baseline, serial, "task-failure-serial");
        assert(active->parallel_tasks == 0 && active->scans == 1);
        active->fail_task = false;
        release_all(reader, error);
        return;
    }
    assert(failed == -1);
    assert(!reader.traversal);
    assert(reader.histories.empty());
    assert(active->parallel_tasks >= 1 && active->scans == 0);
    active->fail_task = false;
    const auto before = active->parallel_tasks;
    assert(ondas_wdb_begin_points(&reader, many.data(), many.size(), error, sizeof error) == 0);
    const auto retried = observe(reader, probe, ticks);
    assert_observations_equal(baseline, retried, "task-failure-retry");
    assert(active->parallel_tasks > before && active->scans == 0);
    release_all(reader, error);
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
    if (s == splitter_name || s == header_name || s == task_name) {
        if (active->missing_parallel) return nullptr;
        if (s == splitter_name) return reinterpret_cast<void*>(event_splitter);
        if (s == header_name) return reinterpret_cast<void*>(event_header_fn);
        return reinterpret_cast<void*>(event_task);
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
void* DataReader::getDataObjMgr() { return nullptr; }
const IObject* DataReader::getRootObject() const { return nullptr; }
void DataReader::traverseAllObjects(
    const std::function<IReader::TraverseCommand(const IObject*)>&, bool, const IBlock*) const {}
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
    reader.index_manager = &manager; reader.common = nullptr; reader.conversion = &conversion;
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
    const std::string mode = argc > 1 ? std::string(argv[1]) : std::string();
    if (mode == "parallel" || mode == "missing-capability" || mode == "omission" ||
        mode == "start-failure" || mode == "task-failure") {
        init_declarations(reader);
        if (mode == "parallel") run_parallel_suite(reader);
        if (mode == "missing-capability") run_missing_capability(reader);
        if (mode == "omission") run_omission(reader);
        if (mode == "start-failure") {
#ifdef ONDAS_WDB_TEST_THREAD_LIMIT
            run_start_failure(reader);
#endif
        }
        if (mode == "task-failure") run_task_failure(reader);
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

    // Logical-close accounting: every close releases all adapter-created SDK indexes exactly
    // once, clears the traversal first, and a later logical session creates and scans those
    // indexes again instead of reusing loaded trees. Repeating the close leaves nothing behind.
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

    // Close/reopen cycles that mirror logical sessions over one retained owner: full read set,
    // subset, disjoint, no query, then the full set again. Each close releases every index and
    // the next session has to create and scan its trees again; a no-query session leaves nothing.
    {
        const std::vector<std::vector<uint64_t>> sessions{{1, 4, 10006}, {1}, {4, 6}, {}, {1, 4, 10006}};
        for (const auto& selected : sessions) {
            auto scans_before = manager.scans;
            if (!selected.empty()) {
                assert(ondas_wdb_begin_points(&reader, selected.data(), selected.size(), error, sizeof(error)) == 0);
                assert(manager.scans > scans_before);
            }
            ondas_wdb_end(&reader);
            const auto session_owned = reader.histories.size();
            assert(!reader.traversal);
            assert(selected.empty() == (session_owned == 0));
            auto before_close = manager.removals;
            assert(ondas_wdb_release_indexes(&reader, error, sizeof(error)) == 0);
            assert(reader.histories.empty());
            assert(manager.removals == before_close + session_owned);
        }
    }

    // Partial delete failure: the failing tree is erased from tracking before its cleanup
    // throws, so a later full release deletes only the rest and never that tree twice. The
    // adapter refuses to park such an owner; the Rust close path retries release and closes it.
    {
        const std::vector<uint64_t> selected{1, 4, 10006};
        assert(ondas_wdb_begin_points(&reader, selected.data(), selected.size(), error, sizeof(error)) == 0);
        ondas_wdb_end(&reader);
        const auto owned = reader.histories.size();
        assert(owned > 1);
        auto before = manager.removals;
        manager.fail_remove_after = before + 1;
        assert(ondas_wdb_release_indexes(&reader, error, sizeof(error)) == -1);
        assert(!reader.traversal);
        const auto remaining = reader.histories.size();
        assert(remaining >= 1 && remaining < owned);
        assert(manager.removals == before + 1);
        manager.fail_remove_after = std::numeric_limits<size_t>::max();
        before = manager.removals;
        assert(ondas_wdb_release_indexes(&reader, error, sizeof(error)) == 0);
        assert(reader.histories.empty());
        assert(manager.removals == before + remaining);
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
