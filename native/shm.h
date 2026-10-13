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
} ondas_shm_decl;
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
    int changed;
} ondas_shm_point;
// No SDK handle survives a call. Native sessions are serialized by Rust.
// Output is Ondas-owned and valid until the next operation on this owner.
// Errors are NUL terminated. Open/read return 0/-1; next returns 1/0/-1.
// On a next error, the output identifies the failed history and source tick.
int ondas_shm_open(const char *, int metadata_only, ondas_shm **, char *, size_t);
void ondas_shm_close(ondas_shm *);
void ondas_shm_metadata(const ondas_shm *, ondas_shm_meta *);
size_t ondas_shm_decl_count(const ondas_shm *);
void ondas_shm_declaration(const ondas_shm *, size_t, ondas_shm_decl *);
// A validated resume requires known entering values and a successful immutable prefix.
int ondas_shm_read(ondas_shm *, const uint64_t *, size_t, uint64_t, uint64_t, int validated_resume, char *, size_t);
// Returns 1 for a complete certified bit batch, or 0 for ordinary-read fallback.
// Descriptors are caller-owned; bytes are Ondas-owned until end/next operation.
// No partial batch may be used after fallback. Direct fallback can prepare owned
// ordinary records for the next matching read; other operations invalidate it.
int ondas_shm_sample_bits(ondas_shm *, const uint64_t *, size_t, uint64_t, int prepare_fallback, ondas_shm_point *);
int ondas_shm_next(ondas_shm *, ondas_shm_record *, char *, size_t);
void ondas_shm_end(ondas_shm *);
#ifdef __cplusplus
}
#endif
#endif
