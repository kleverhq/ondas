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
} ondas_shm_record;
typedef struct {
    uint64_t changed_at;
    const uint8_t *data;
    size_t len;
    int changed, certified;
} ondas_shm_point;
// Private work accounting, read only by backend tests. Counts work performed
// before Rust callbacks as well as selected-route enumeration and source opens.
typedef struct {
    uint64_t sessions, hierarchy_nodes, histories, history_records;
} ondas_shm_work;
// No SDK handle survives a call. Native sessions are serialized by Rust.
// Output is Ondas-owned and valid until the next operation on this owner.
// Errors are NUL terminated. Open/read return 0/-1; next returns 1/0/-1.
// On a next error, the output identifies the failed history and source tick.
int ondas_shm_open(const char *, int metadata_only, ondas_shm **, char *, size_t);
void ondas_shm_close(ondas_shm *);
void ondas_shm_metadata(const ondas_shm *, ondas_shm_meta *);
void ondas_shm_work_done(const ondas_shm *, ondas_shm_work *);
size_t ondas_shm_decl_count(const ondas_shm *);
void ondas_shm_declaration(const ondas_shm *, size_t, ondas_shm_decl *);
void ondas_shm_enum_variant(const ondas_shm *, size_t declaration, size_t variant, ondas_shm_variant *);
// A validated resume requires known entering values and a successful immutable prefix.
int ondas_shm_read(ondas_shm *, const uint64_t *, size_t, uint64_t, uint64_t, int validated_resume, char *, size_t);
// Returns 1 for a complete certified bit batch, 2 for a partial batch, or 0
// for ordinary-read fallback. Certified descriptors have certified == 1 and bytes.
// Unqualified descriptors are zero-filled (certified, data, len, changed and
// changed_at are zero) and require ordinary history. No other flag is valid.
// Descriptors are caller-owned; bytes are Ondas-owned until end/next operation.
// Partial success always prepares ordinary rows for only the unqualified IDs,
// in input order. With no certified values, prepare_fallback controls preparation.
// A matching read consumes those rows; other operations invalidate them.
int ondas_shm_sample_bits(ondas_shm *, const uint64_t *, size_t, uint64_t, int prepare_fallback, ondas_shm_point *);
int ondas_shm_next(ondas_shm *, ondas_shm_record *, char *, size_t);
void ondas_shm_end(ondas_shm *);
#ifdef __cplusplus
}
#endif
#endif
