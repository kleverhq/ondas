#ifndef ONDAS_WLF_H
#define ONDAS_WLF_H
#include <stddef.h>
#include <stdint.h>
/* Owned by the worker thread. No vendor type crosses this interface.
 * Copy borrowed strings and record bytes before the next shim call.
 * End each traversal on all exit paths; close also releases active traversal. */
typedef struct ondas_wlf ondas_wlf;
typedef struct {
    uint64_t first, last;
    uint32_t resolution;
    const char *writer;
} ondas_wlf_meta;
typedef struct {
    int64_t id;
    size_t parent;
    uint32_t entry, encoding, width, direction, is_constant, has_range;
    uint32_t signedness, logic_domain;
    int64_t msb, lsb;
    const char *name, *kind, *definition;
    uint32_t verilog_name; /* Nonzero for SDK-identified Verilog symbols. */
} ondas_wlf_decl;
typedef struct {
    size_t selection;
    uint64_t tick, occurrences;
    const uint8_t *data;
    size_t len;
    uint64_t real_bits;
} ondas_wlf_record;
/* Errors: -1 reader, -2 malformed, -3 initialization unavailable. */
int ondas_wlf_open(const char *, const char *, int, ondas_wlf **, char *, size_t);
void ondas_wlf_close(ondas_wlf *);
void ondas_wlf_metadata(ondas_wlf *, ondas_wlf_meta *);
size_t ondas_wlf_decl_count(ondas_wlf *);
void ondas_wlf_declaration(ondas_wlf *, size_t, ondas_wlf_decl *);
int ondas_wlf_begin(ondas_wlf *, const size_t *, size_t, uint64_t, uint64_t, char *, size_t);
int ondas_wlf_next(ondas_wlf *, ondas_wlf_record *, char *, size_t);
/* Owned completed-tick batch: 0 has records or an error, 1 is drained,
 * 2 is drained and finished. Does not advance the SDK or invalidate bytes. */
int ondas_wlf_batch_end(ondas_wlf *);
void ondas_wlf_end(ondas_wlf *);
void ondas_wlf_resources(ondas_wlf *, size_t *, size_t *, int *);
#endif
