#ifndef ONDAS_VWDB_H
#define ONDAS_VWDB_H
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct ondas_vwdb ondas_vwdb;
typedef struct {
    uint64_t first, last;
    int32_t exponent;
} ondas_vwdb_meta;
typedef struct {
    size_t parent;
    const char *name, *kind;
    int32_t native_kind, encoding;
    uint32_t width, has_range, is_constant, indexed_children;
    int64_t msb, lsb;
    size_t variant_count;
} ondas_vwdb_decl;
typedef struct { const char *encoded, *label; } ondas_vwdb_variant;
typedef struct {
    size_t declaration;
    uint64_t tick;
    int32_t encoding;
    const uint8_t *data;
    size_t len;
} ondas_vwdb_record;

int ondas_vwdb_ready(const char *, char *, size_t);
int ondas_vwdb_open(const char *, const char *, int, ondas_vwdb **, char *, size_t);
void ondas_vwdb_close(ondas_vwdb *);
void ondas_vwdb_metadata(const ondas_vwdb *, ondas_vwdb_meta *);
size_t ondas_vwdb_decl_count(const ondas_vwdb *);
void ondas_vwdb_declaration(const ondas_vwdb *, size_t, ondas_vwdb_decl *);
void ondas_vwdb_enum_variant(const ondas_vwdb *, size_t, size_t, ondas_vwdb_variant *);
int ondas_vwdb_read(ondas_vwdb *, const size_t *, size_t, uint64_t, char *, size_t);
int ondas_vwdb_next(ondas_vwdb *, ondas_vwdb_record *, char *, size_t);
void ondas_vwdb_end(ondas_vwdb *);

#ifdef __cplusplus
}
#endif
#endif
