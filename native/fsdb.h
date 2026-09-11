#ifndef ONDAS_FSDB_H
#define ONDAS_FSDB_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Private ABI. No SDK definitions cross this boundary. All calls are serialized
 * by the Rust adapter. No C++ exception escapes. Strings are UTF-8 names, while
 * waveform string values are byte sequences. Borrowed output lasts until the
 * next call on its owner; Rust copies it before unlocking or calling visitors. */
typedef struct ondas_fsdb ondas_fsdb;
enum { OFS_UNSUPPORTED, OFS_BITS, OFS_REAL, OFS_STRING, OFS_EVENT };
enum { OFS_SCOPE, OFS_UPSCOPE, OFS_VAR };

typedef struct {
    uint64_t id;
    uint32_t entry, encoding, width, direction, is_constant, has_range, packing;
    int64_t msb, lsb;
    const char *name, *kind, *definition;
} ondas_fsdb_decl;

typedef struct {
    uint64_t first, last;
    const char *scale, *writer, *date;
} ondas_fsdb_meta;

typedef struct {
    uint64_t tick, id;
    uint32_t encoding;
    double real;
    const uint8_t *data;
    size_t len;
} ondas_fsdb_value;

/* Errors are copied into the caller's NUL-terminated error buffer. Open/probe/
 * begin return 0 on success, -1 on error. Next returns 1 for a record, 0 for EOF,
 * -1 on error. Closing also releases an active traversal. */
int ondas_fsdb_probe(const char *, int *, char *, size_t);
int ondas_fsdb_open(const char *, ondas_fsdb **, char *, size_t);
void ondas_fsdb_close(ondas_fsdb *);
void ondas_fsdb_metadata(ondas_fsdb *, ondas_fsdb_meta *);
size_t ondas_fsdb_decl_count(ondas_fsdb *);
void ondas_fsdb_declaration(ondas_fsdb *, size_t, ondas_fsdb_decl *);
int ondas_fsdb_begin(ondas_fsdb *, const uint64_t *, size_t, char *, size_t);
int ondas_fsdb_next(ondas_fsdb *, ondas_fsdb_value *, char *, size_t);
void ondas_fsdb_end(ondas_fsdb *);

#ifdef __cplusplus
}
#endif
#endif
