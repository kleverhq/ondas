#ifndef ONDAS_WDB_H
#define ONDAS_WDB_H
#include <stddef.h>
#include <stdint.h>
#ifdef __cplusplus
extern "C" {
#endif
struct ondas_wdb_meta {
    uint64_t last;
    int32_t precision;
    uint32_t has_variables;
};
struct ondas_wdb_decl {
    uint64_t id;
    size_t parent;
    uint32_t scope, encoding, width, has_range, is_constant;
    uint32_t direction, signedness, logic_domain, packing;
    uint32_t name_syntax; /* 0: unknown, 1: Verilog, 2: VHDL */
    int64_t msb, lsb;
    const char *name, *reader_name, *kind, *type_name, *definition;
    size_t variants, base_len;
};
struct ondas_wdb_record {
    uint64_t tick, id;
    uint32_t encoding;
    const unsigned char *data;
    size_t len;
};
int ondas_wdb_open(const char*, int, void**, char*, size_t);
void ondas_wdb_close(void*);
void ondas_wdb_metadata(void*, struct ondas_wdb_meta*);
size_t ondas_wdb_decl_count(void*);
size_t ondas_wdb_scope_count(void*);
int ondas_wdb_declaration(void*, size_t, struct ondas_wdb_decl*);
int ondas_wdb_enum_variant(void*, size_t, size_t, const char**, const char**);
int ondas_wdb_begin(void*, const uint64_t*, size_t, uint64_t, uint64_t, char*, size_t);
int ondas_wdb_sample_bits(void*, uint64_t, uint64_t, uint32_t, uint32_t,
                          struct ondas_wdb_record*, int*, char*, size_t);
int ondas_wdb_next(void*, struct ondas_wdb_record*, char*, size_t);
void ondas_wdb_end(void*);
#ifdef __cplusplus
}
#endif
#endif
