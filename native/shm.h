#ifndef ONDAS_SHM_H
#define ONDAS_SHM_H
#include <stddef.h>
#include <stdint.h>
#ifdef __cplusplus
extern "C" {
#endif
typedef struct ondas_shm ondas_shm;
typedef struct {
    uint64_t id;
    uint32_t entry, encoding, width, has_range, is_constant;
    int64_t msb, lsb;
    const char *name, *kind, *definition;
    size_t variant_count;
    const char *enum_name;
} ondas_shm_decl;
typedef struct {
    const char *encoded, *label;
} ondas_shm_variant;
typedef struct {
    uint64_t first, last;
    int32_t exponent;
} ondas_shm_meta;
typedef struct {
    uint64_t id, tick;
    uint32_t encoding;
    const uint8_t *data;
    size_t len;
    uint64_t occurrences;
} ondas_shm_record;
typedef struct {
    uint64_t changed_at;
    const uint8_t *data;
    size_t len;
    int changed, certified;
} ondas_shm_point;
// Private work accounting, read only by backend tests. Counts work performed
// before Rust callbacks as well as selected-route enumeration and source opens.
// Reopens/reloaded histories count restoration separately from initial operations.
// Pending records include allocated batch slots and transient records; payload is
// retained string capacity, including heads, batch and a decoded replacement.
// These counters do not measure SDK allocations or the common Rust engine.
typedef struct {
    uint64_t sessions, hierarchy_nodes, histories, history_records;
    uint64_t delivered, first_delivery_records, pending_records, pending_payload;
    uint64_t reopens, reloaded_histories, replayed_records;
} ondas_shm_work;
// Rust serializes native calls. An active stream owns its cursors/session;
// another session suspends them before changing shared SDK state.
// Output is Ondas-owned and valid until the next operation on this owner.
// Errors are NUL terminated. Open/read return 0 on success, -1 for a reader
// failure, -2 for a POSIX error (os_error), -3 for unavailable initialization.
// Open/read always initialize os_error. Next returns 1/0/-1/-2/-3/-4.
// -1 is a timed payload error. Other negative next codes have no source tick;
// -2 preserves os_error, -3 means unavailable, -4 is an unknown traversal error.
// On a timed next error, output identifies the failed history and source tick.
int ondas_shm_open(const char *, int metadata_only, ondas_shm **, char *, size_t, int *os_error);
void ondas_shm_close(ondas_shm *);
void ondas_shm_metadata(const ondas_shm *, ondas_shm_meta *);
void ondas_shm_work_done(const ondas_shm *, ondas_shm_work *);
size_t ondas_shm_decl_count(const ondas_shm *);
void ondas_shm_declaration(const ondas_shm *, size_t, ondas_shm_decl *);
void ondas_shm_enum_variant(const ondas_shm *, size_t declaration, size_t variant, ondas_shm_variant *);
// A validated resume requires known entering values and a successful immutable prefix.
int ondas_shm_read(ondas_shm *, const uint64_t *, size_t, uint64_t, uint64_t, int validated_resume, char *, size_t, int *os_error);
// Returns 1 for a complete certified bit batch, 2 for a partial batch, or 0
// for ordinary-read fallback. Certified descriptors have certified == 1 and bytes.
// Unqualified descriptors are zero-filled (certified, data, len, changed and
// changed_at are zero) and require ordinary history. No other flag is valid.
// Descriptors are caller-owned; bytes are Ondas-owned until end/next operation.
// Partial success always prepares a bounded traversal for only the unqualified IDs,
// in input order. With no certified values, prepare_fallback controls preparation.
// A matching read consumes that traversal; other operations invalidate it.
int ondas_shm_sample_bits(ondas_shm *, const uint64_t *, size_t, uint64_t, int prepare_fallback, ondas_shm_point *);
int ondas_shm_next(ondas_shm *, ondas_shm_record *, char *, size_t, int *os_error);
void ondas_shm_end(ondas_shm *);
#ifdef __cplusplus
}
#endif
#endif
