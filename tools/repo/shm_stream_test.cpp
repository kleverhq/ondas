// Exercise the real merge/decoder against a controlled private ABI source.
// This is not SDK qualification: real-runtime conformance is separate.
template<class S, class R> void check_account(const S &, const R *);
#define ONDAS_SHM_TEST_ACCOUNT(stream, transient) check_account(stream, transient)
#include "../../native/shm.cpp"
#undef ONDAS_SHM_TEST_ACCOUNT
#include <cassert>
#include <cstdarg>
#include <iostream>

namespace {
struct Row {
    SstTime time;
    int state;
    std::string data;
    double real = 0;
};
struct Input {
    std::vector<Row> rows;
    size_t burst = 0, sequential = 0, width = 1;
    bool throw_advance = false;
    Row at(size_t index) const {
        if (sequential) return {{index, 0}, 2, std::string(width, index % 2 ? '1' : '0')};
        if (burst) return {{index < burst ? 1ULL : 2ULL, index}, 2, index % 2 ? "1" : "0"};
        return rows.at(index);
    }
    size_t size() const { return sequential ? sequential : burst ? burst + 1 : rows.size(); }
};
struct Position { Input *input; size_t index = 0; std::string buffer; };
size_t loads = 0, unloads = 0, live = 0;
struct Unwind {};
Position &position(SstHandle handle) { return *reinterpret_cast<Position *>(handle); }
SstHandle variable(Input &input) { return reinterpret_cast<SstHandle>(&input); }
Declaration declaration(Input &input, std::uint64_t id, std::uint32_t encoding = 1) {
    Declaration d;
    d.variable = variable(input);
    d.descriptor.id = id; d.descriptor.encoding = encoding; d.descriptor.width = 1;
    return d;
}
std::uint64_t head_capacity(const Stream &stream) {
    std::uint64_t sum = 0;
    for (const auto &history : stream.histories) sum += history.head.data.capacity();
    return sum;
}
struct CapacityOracle;
CapacityOracle *observing = nullptr;
struct CapacityOracle {
    ondas_shm &reader;
    std::uint64_t peak, slots;
    size_t observations = 0, error_transients = 0;
    explicit CapacityOracle(ondas_shm &value)
        : reader(value), peak(value.work.pending_payload), slots(value.work.pending_records) {
        assert(!observing); observing = this;
    }
    ~CapacityOracle() { observing = nullptr; }
    std::uint64_t batch_capacity() const {
        std::uint64_t sum = 0;
        for (const auto &record : reader.batch) sum += record.data.capacity();
        return sum;
    }
    void check_current() const {
        const auto &stream = *reader.stream;
        assert(stream.head_payload == head_capacity(stream));
        assert(stream.queued_payload == batch_capacity());
        assert(stream.work.pending_payload >= head_capacity(stream) + batch_capacity() + stream.output.data.capacity());
    }
};
void prepare(ondas_shm &reader, const std::vector<Declaration> &declarations) {
    assert(!reader.stream && reader.batch.empty());
    reader.declarations = declarations;
    std::vector<std::uint64_t> ids;
    for (const auto &d : declarations) ids.push_back(d.descriptor.id);
    reader.stream = std::make_unique<Stream>(reader.path, reader.declarations, reader.source_nodes,
        reader.work, ids.data(), ids.size(), 0, 100, false);
    reader.stream->declarations = declarations;
    for (const auto &d : declarations) reader.stream->cursors.push_back(std::make_unique<Cursor>(d.variable));
    active_stream = reader.stream.get(); reader.stream->activated = true;
    assert(reader.stream->head_payload == head_capacity(*reader.stream));
}
}
template<class S, class R> void check_account(const S &stream, const R *transient) {
    if (!observing || observing->reader.stream.get() != &stream) return;
    auto &oracle = *observing;
    const auto heads = head_capacity(stream);
    const auto batch = oracle.batch_capacity();
    const auto payload = heads + batch + stream.output.data.capacity() + (transient ? transient->data.capacity() : 0);
    oracle.peak = std::max(oracle.peak, payload);
    oracle.slots = std::max<std::uint64_t>(oracle.slots, stream.histories.capacity() + oracle.reader.batch.capacity() + 2);
    assert(stream.head_payload == heads);
    assert(stream.queued_payload == batch);
    assert(stream.work.pending_payload == oracle.peak);
    assert(stream.work.pending_records == oracle.slots);
    ++oracle.observations;
    if (transient && transient->failed) ++oracle.error_transients;
}
extern "C" {
int sst2Init() { return 1; }
int drOpenFile(const char *, SstHandle *, int (*)(void *, int), void *) { std::abort(); }
int drClose(SstHandle) { std::abort(); }
int da_get(int property, SstHandle handle) {
    if (property == 0x12) return 0;
    assert(property == 0x1a3);
    auto &p = position(handle); return p.input->at(p.index).state;
}
int da_get_int(int, SstHandle, int *) { std::abort(); }
int da_get_real(int property, SstHandle handle, double *out) {
    assert(property == 0x107); auto &p = position(handle); *out = p.input->at(p.index).real; return 0;
}
const char *da_get_sstr(int property, SstHandle handle) {
    assert(property == 0x15a); auto &p = position(handle); p.buffer = p.input->at(p.index).data; return p.buffer.c_str();
}
const char *da_get_sstr_pl(int property, SstHandle handle, ...) { return da_get_sstr(property, handle); }
int da_get_xvalseq(int property, SstHandle handle, SstTime *out) {
    auto &p = position(handle);
    if (property == 0x136) *out = p.input->at(p.index).time;
    else {
        assert(property == 0x146);
        *out = p.index + 1 < p.input->size() ? p.input->at(p.index + 1).time : SstTime{UINT64_MAX, UINT64_MAX};
    }
    return 0;
}
SstHandle da_handle(int property, SstHandle handle) {
    if (property == 0x12f) { ++loads; return 1; }
    if (property == 0x13f) { ++unloads; return 1; }
    assert(property == 0x17f);
    auto &p = position(handle);
    if (p.input->throw_advance) { p.input->throw_advance = false; throw Unwind{}; }
    return ++p.index < p.input->size() ? 1 : 0;
}
SstHandle da_handle_pl(int property, SstHandle handle, ...) {
    assert(property == 0x22e);
    ++live;
    return reinterpret_cast<SstHandle>(new Position{reinterpret_cast<Input *>(handle), 0, {}});
}
SstHandle da_iterate(int, SstHandle) { std::abort(); }
SstHandle da_scan(SstHandle) { std::abort(); }
void da_delete(SstHandle handle) { delete &position(handle); --live; }
}

namespace {
void burst_and_merge() {
    Input burst{{}, 1'000'000};
    Input sparse{{{{0, 0}, 2, "0"}, {{2, 1}, 2, "1"}}};
    std::vector<Declaration> declarations{declaration(burst, 9), declaration(sparse, 2)};
    std::vector<SourceNode> nodes;
    ondas_shm_work work{};
    std::string path;
    const std::uint64_t ids[]{9, 2};
    Stream stream(path, declarations, nodes, work, ids, 2, 0, 100, false);
    stream.declarations = declarations;
    for (auto &d : declarations) stream.cursors.push_back(std::make_unique<Cursor>(d.variable));
    active_stream = &stream;
    stream.activated = true;
    std::vector<std::pair<std::uint64_t,std::uint64_t>> output;
    while (const auto *record = stream.next()) {
        assert(!record->failed);
        output.emplace_back(record->tick, record->id);
        if (record->tick == 1) assert(record->data == "1");
    }
    assert((output == std::vector<std::pair<std::uint64_t,std::uint64_t>>{{0,2},{1,9},{2,2},{2,9}}));
    assert(work.history_records == 1'000'003);
    assert(work.pending_records == 4);
    assert(work.pending_payload < 128);
    std::cout << "million-write tick: records=" << work.history_records
              << " pending_slots=" << work.pending_records << " payload_capacity=" << work.pending_payload << '\n';
}
void batch_bounds() {
    for (size_t width : {1U, 4096U, 131072U}) {
        Input input{{}, 0, 1'000'000, width};
        ondas_shm reader;
        reader.declarations.push_back(declaration(input, 0));
        reader.declarations[0].descriptor.width = width;
        const std::uint64_t ids[]{0};
        reader.stream = std::make_unique<Stream>(reader.path, reader.declarations,
            reader.source_nodes, reader.work, ids, 1, 0, 1'000'000, false);
        reader.stream->declarations = reader.declarations;
        reader.stream->cursors.push_back(std::make_unique<Cursor>(variable(input)));
        active_stream = reader.stream.get(); reader.stream->activated = true;
        ondas_shm_record record{};
        char error[256]{};
        int os_error = -1;
        assert(ondas_shm_next(&reader, &record, error, sizeof(error), &os_error) == 1);
        assert(os_error == 0);
        assert(record.tick == 0 && record.len == width);
        assert(reader.work.history_records <= 65);
        assert(reader.work.pending_records <= 67);
        assert(reader.batch.size() <= 64);
        assert(reader.work.pending_payload <= 65536 + 4 * width + 128);
        std::cout << "batch width=" << width << " records=" << reader.work.history_records
                  << " rows=" << reader.batch.size() << " payload_capacity=" << reader.work.pending_payload << '\n';
        if (width == 1) {
            for (size_t i = 1; i <= 64; ++i) {
                assert(ondas_shm_next(&reader, &record, error, sizeof(error), &os_error) == 1);
                assert(record.tick == i);
            }
            assert(reader.batch.size() == 1024);
            assert(reader.work.history_records <= 1089);
            assert(reader.work.pending_records <= 1027);
            assert(reader.work.pending_payload <= 65536 + 128);
        }
        ondas_shm_end(&reader);
        assert(live == 0 && active_stream == nullptr && !reader.stream && reader.batch.capacity() == 0);
    }
}
void events_and_failures() {
    for (const auto encoding : {1U, 4U}) {
        Input input{{}, 1'000'000};
        auto d = declaration(input, 0, encoding);
        Cursor cursor(d.variable);
        History h;
        ondas_shm_work work{};
        size_t peak = 0;
        h.fill(d, cursor, 1, work, [&](const Record *r) {
            peak = std::max(peak, h.head.data.capacity() + (r ? r->data.capacity() : 0));
        });
        assert(h.ready && h.done && !h.head.failed && h.head.tick == 1);
        assert(work.history_records == 1'000'000 && peak < 64);
        if (encoding == 4) assert(h.head.occurrences == 1'000'000);
    }
    for (Input input : {
        Input{{{{0,0}, 2, "0"}, {{1,0}, 2, "1"}, {{0,1}, 2, "0"}}},
        Input{{{{0,0}, 2, "0"}, {{1,1}, 2, "1"}, {{1,0}, 2, "0"}}},
        Input{{{{0,0}, 2, "0"}, {{1,0}, -1, ""}}},
        Input{{{{0,0}, 2, "0"}, {{1,0}, 2, "?"}, {{1,1}, 2, "1"}}},
    }) {
        auto d = declaration(input, 0);
        Cursor cursor(d.variable);
        History h;
        ondas_shm_work work{};
        do {
            h.ready = false;
            h.fill(d, cursor, 100, work, [](const Record *) {});
        } while (!h.done);
        assert(h.ready && h.head.failed);
    }
    Input missing{{{{0,0}, -1, ""}, {{5,0}, 2, "1"}}};
    auto d = declaration(missing, 0);
    Cursor cursor(d.variable);
    History h;
    ondas_shm_work work{};
    h.fill(d, cursor, 10, work, [](const Record *) {});
    assert(h.ready && !h.head.failed && h.head.tick == 5);
}
void strings_and_reals() {
    Input input{{{{0,0}, 2, "\"good\""}, {{1,0}, 2, "\"\\xFF\""}, {{1,1}, 2, "\"valid\""}}};
    auto d = declaration(input, 0, 3);
    Cursor cursor(d.variable);
    History h;
    ondas_shm_work work{};
    h.fill(d, cursor, 2, work, [](const Record *) {});
    assert(h.head.data == "good");
    h.ready = false;
    h.fill(d, cursor, 2, work, [](const Record *) {});
    assert(h.done && !h.head.failed && h.head.data == std::string(1, char(255)));
    Input real{{{{0,0}, 2, "", 0.0}, {{0,1}, 2, "", -0.0}, {{1,0}, 2, "", 1.5}}};
    auto r = declaration(real, 1, 2);
    Cursor real_cursor(r.variable);
    History rh;
    rh.fill(r, real_cursor, 1, work, [](const Record *) {});
    double expected = -0.0;
    assert(rh.head.data.size() == sizeof(double));
    assert(std::memcmp(rh.head.data.data(), &expected, sizeof(double)) == 0);
}
void mixed_capacity_and_repeated_reads() {
    const std::string huge(131072, 'a');
    Input bits{{{{0,0}, 2, "0"}, {{2,0}, 2, "1"}}};
    Input real{{{{0,0}, 2, "", 0.0}, {{0,1}, 2, "", -0.0}, {{2,0}, 2, "", 1.5}}};
    Input strings{{{{0,0}, 2, "\"" + std::string(32768, 'b') + "\""},
                   {{0,1}, 2, "\"ok\""}, {{1,0}, 2, "\"" + huge + "\""}, {{2,0}, 2, "\"\""}}};
    Input events{{{{0,0}, 1, ""}, {{1,0}, 2, ""}, {{1,1}, 2, ""}, {{2,0}, 1, ""}, {{3,0}, 2, ""}}};
    ondas_shm reader;
    prepare(reader, {declaration(bits,1), declaration(real,2,2), declaration(strings,3,3), declaration(events,4,4)});
    CapacityOracle oracle(reader);
    std::vector<Record> records;
    ondas_shm_record out{};
    char error[256]{};
    int os_error = -1;
    for (;;) {
        const int code = ondas_shm_next(&reader, &out, error, sizeof(error), &os_error);
        assert(code >= 0 && os_error == 0);
        oracle.check_current();
        if (!code) break;
        records.push_back({out.id,out.tick,0,out.encoding,false,
            out.len ? std::string(reinterpret_cast<const char *>(out.data),out.len) : std::string(),out.occurrences});
        if (records.size() == 4) {
            assert(reader.position == reader.batch.size());
            assert(oracle.batch_capacity() >= huge.size());
        }
    }
    const std::vector<std::pair<std::uint64_t,std::uint64_t>> expected{{0,1},{0,2},{0,3},{1,3},{1,4},{2,1},{2,2},{2,3},{3,4}};
    std::vector<std::pair<std::uint64_t,std::uint64_t>> order;
    for (const auto &record : records) order.emplace_back(record.tick,record.id);
    assert(order == expected);
    assert(records[0].data == "0" && records[2].data == "ok" && records[3].data == huge);
    double negative_zero = -0.0, one_and_half = 1.5;
    assert(records[1].data.size() == sizeof(double) && std::memcmp(records[1].data.data(),&negative_zero,sizeof(double)) == 0);
    assert(records[6].data.size() == sizeof(double) && std::memcmp(records[6].data.data(),&one_and_half,sizeof(double)) == 0);
    assert(records[4].occurrences == 2 && records[8].occurrences == 1 && records[7].data.empty());
    assert(oracle.observations > 20 && oracle.error_transients == 0);
    const auto peak = reader.work.pending_payload, slots = reader.work.pending_records;
    ondas_shm_end(&reader);
    assert(!reader.stream && reader.batch.capacity() == 0 && live == 0);
    Input small{{{{0,0}, 2, "1"}}};
    prepare(reader, {declaration(small,1)});
    assert(ondas_shm_next(&reader,&out,error,sizeof(error),&os_error) == 1);
    oracle.check_current();
    assert(reader.work.pending_payload == peak && reader.work.pending_records == slots);
    ondas_shm_end(&reader);
    assert(reader.work.pending_payload == peak && reader.work.pending_records == slots);
    std::cout << "independent capacity oracle: observations=" << oracle.observations << " peak=" << peak << '\n';
}
void error_replacement_peak() {
    const std::string large(131072, 'x');
    Input input{{{{0,0}, 2, "\"" + large + "\""}, {{0,1}, 2, "unterminated"}, {{1,0}, 2, "\"valid\""}}};
    auto d = declaration(input,9,3);
    Cursor cursor(d.variable);
    History history;
    ondas_shm_work work{};
    bool saw_error_temporary = false;
    size_t coexistence = 0;
    history.fill(d,cursor,10,work,[&](const Record *value) {
        if (value && value->failed) {
            saw_error_temporary = true;
            assert(history.head.data == large);
            coexistence = history.head.data.capacity() + value->data.capacity();
        }
    });
    assert(saw_error_temporary && coexistence > large.size());
    assert(history.ready && history.done && history.head.failed && history.head.tick == 0 && work.history_records == 2);
    Input earlier{{{{0,0}, 2, "0"}, {{2,0}, 2, "1"}}};
    Input bad{{{{1,0}, 2, "\"" + large + "\""}, {{1,1}, 2, "bad"}}};
    ondas_shm reader;
    prepare(reader,{declaration(earlier,1),declaration(bad,9,3)});
    CapacityOracle oracle(reader);
    ondas_shm_record out{}; char error[256]{}; int os_error = -1;
    assert(ondas_shm_next(&reader,&out,error,sizeof(error),&os_error) == 1 && out.id == 1 && out.tick == 0);
    assert(ondas_shm_next(&reader,&out,error,sizeof(error),&os_error) == -1 && out.id == 9 && out.tick == 1 && os_error == 0);
    assert(std::string(error).find("history 9 at tick 1") == 0);
    assert(oracle.error_transients == 1 && reader.work.pending_payload > large.size());
    oracle.check_current();
    ondas_shm_end(&reader);
}
void unwind_and_suspend_capacity() {
    const std::string large(8192,'z');
    Input input{{{{0,0}, 2, "\"" + large + "\""}, {{1,0}, 2, "\"next\""}}};
    input.throw_advance = true;
    ondas_shm reader;
    prepare(reader,{declaration(input,1,3)});
    CapacityOracle oracle(reader);
    bool caught = false;
    try { reader.stream->fill(0); }
    catch (const Unwind &) { caught = true; }
    assert(caught && reader.stream->histories[0].head.data == large);
    oracle.check_current();
    const auto retained = head_capacity(*reader.stream);
    const auto peak = reader.work.pending_payload;
    suspend_active();
    assert(active_stream == nullptr && reader.stream->cursors.empty());
    assert(reader.stream->histories[0].head.data == large && reader.stream->head_payload == retained);
    assert(reader.work.pending_payload == peak);
    oracle.check_current();
    ondas_shm_end(&reader);
    assert(live == 0 && !reader.stream && reader.batch.capacity() == 0 && reader.work.pending_payload == peak);
}
void empty_and_finished_heads() {
    Input finished, value{{{{0,0},2,"1"}}};
    ondas_shm reader;
    prepare(reader,{declaration(finished,1),declaration(value,2)});
    reader.stream->histories[0].done = true;
    CapacityOracle oracle(reader);
    ondas_shm_record out{}; char error[256]{}; int os_error = -1;
    assert(ondas_shm_next(&reader,&out,error,sizeof(error),&os_error) == 1 && out.id == 2 && out.tick == 0);
    assert(ondas_shm_next(&reader,&out,error,sizeof(error),&os_error) == 0);
    oracle.check_current();
    assert(reader.work.history_records == 1 && oracle.observations > 0);
    ondas_shm_end(&reader);
    std::string path;
    std::vector<Declaration> declarations;
    std::vector<SourceNode> nodes;
    ondas_shm_work work{};
    const std::uint64_t unused = 0;
    Stream empty(path,declarations,nodes,work,&unused,0,0,10,false);
    active_stream = &empty; empty.activated = true;
    assert(!empty.next() && empty.head_payload == 0);
    empty.account();
    assert(work.pending_payload == empty.output.data.capacity() && work.pending_records == 2);
}
}
int main() {
    burst_and_merge();
    batch_bounds();
    events_and_failures();
    strings_and_reals();
    mixed_capacity_and_repeated_reads();
    error_replacement_peak();
    unwind_and_suspend_capacity();
    empty_and_finished_heads();
    assert(live == 0 && loads == unloads);
    std::cout << "cursor cleanup: loads=" << loads << " unloads=" << unloads << " live=" << live << '\n';
}
