/* Private consumer of the installed WLF API; all SDK calls run on one thread. */
#include "wlf.h"
#include <inttypes.h>
#include <limits.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "wlf_api.h"
#include "wlf_enum.h"

/* The SDK retains expression lookup pointers after their owner closes. Clear
 * that optional lookup state at owner boundaries, and select the live owner
 * for reads and cleanup. These exports are absent from the SDK header. */
extern WlfFileId setCurrentNamespace(WlfFileId file);
extern WlfFileId wlfFileOpenByMode(char *path, char *name, int mode);
extern void *current_expr_contxt, *current_expr_objtd;
#define WLF_LITE_OPEN 7

_Static_assert(sizeof(WlfTime64) == 8 && (WlfTime64)-1 < 0, "signed 64-bit SDK time required");
_Static_assert(sizeof(double) == 8, "binary64 SDK real required");
_Static_assert(sizeof(size_t) == 8, "64-bit private ABI required");
_Static_assert(sizeof(ondas_wlf_meta) == 32 && sizeof(ondas_wlf_decl) == 96 && sizeof(ondas_wlf_record) == 48, "private ABI layout mismatch");
_Static_assert(offsetof(ondas_wlf_decl, name) == 64 && offsetof(ondas_wlf_record, data) == 24, "private ABI offset mismatch");

typedef struct {
    char *bits, *label;
} enum_variant;
typedef struct {
    ondas_wlf_decl out;
    WlfSymbolId symbol;
    unsigned datatype;
    int size;
    size_t enum_count;
    enum_variant *variants;
} declaration;
typedef struct {
    struct ondas_wlf *owner;
    declaration *decl;
    WlfValueId value;
    uint8_t *data;
    size_t len;
    uint64_t real_bits, occurrences;
    int present;
} selected;
typedef struct {
    ondas_wlf_record out;
    size_t offset;
} buffered_record;
#define WLF_BATCH_RECORDS 256
#define WLF_BATCH_BYTES (64 * 1024)
struct ondas_wlf {
    WlfFileId file;
    WlfFileInfo info;
    declaration *decls;
    size_t count, capacity;
    WlfPackId pack;
    selected *selected;
    size_t selected_count, cursor;
    buffered_record *records;
    size_t record_count, record_capacity;
    uint8_t *record_bytes;
    size_t record_bytes_len, record_bytes_capacity;
    WlfTime64 start, end, tick, next_tick;
    int finished, stopped, have_tick;
    char failure[512];
};
static int initialized;
static WlfFileId namespace_guard;
static uint64_t next_guard;
static int fail(char *error, size_t cap, int code, const char *message) {
    if (cap) snprintf(error, cap, "%s", message ? message : "WLF SDK failure");
    return code;
}
static int sdk_error(char *error, size_t cap) {
    return fail(error, cap, -1, wlfErrorMsg());
}
static char *copy(const char *text) {
    if (!text) return NULL;
    size_t n = strlen(text) + 1;
    char *result = malloc(n);
    if (result) memcpy(result, text, n);
    return result;
}
static const char *scope_kind(WlfSymbolSel64 k) {
    if (k & wlfSelModule) return "module";
    if (k & wlfSelArchitecture) return "architecture";
    if (k & wlfSelTask) return "task";
    if (k & wlfSelFunction) return "function";
    if (k & (wlfSelGenerate | wlfSelVlGenerateBlock)) return "generate";
    if (k & (wlfSelPackage | wlfSelVlPackage)) return "package";
    if (k & wlfSelSVInterface) return "interface";
    if (k & wlfSelBlock) return "block";
    if (k & wlfSelProcess) return "process";
    return NULL;
}
static const char *variable_kind(WlfSymbolSel64 k) {
    if (k & wlfSelParameter) return "parameter";
    if (k & wlfSelConstant) return "constant";
    if (k & wlfSelGeneric) return "generic";
    if (k & wlfSelNamedEvent) return "event";
    if (k & wlfSelInteger) return "integer";
    if (k & wlfSelTime) return "time";
    if (k & wlfSelReal) return "real";
    if (k & wlfSelNet) return "wire";
    if (k & wlfSelReg) return "reg";
    if (k & wlfSelVariable) return "variable";
    if (k & wlfSelSignal) return "signal";
    if (k & wlfSelAlias) return "alias";
    return NULL;
}
static void clear_enum(declaration *d) {
    for (size_t i = 0; i < d->enum_count; ++i) {
        free(d->variants[i].bits); free(d->variants[i].label);
    }
    free(d->variants); d->variants = NULL; d->enum_count = 0;
}
static int add_variant(declaration *d, size_t index, const uint8_t *bits, const char *label) {
    enum_variant *v = &d->variants[index];
    v->bits = malloc((size_t)d->out.width + 1); v->label = copy(label);
    if (!v->bits || !v->label) return -1;
    memcpy(v->bits, bits, d->out.width); v->bits[d->out.width] = 0;
    return 0;
}
static int read_sv_enum(declaration *d, WlfTypeId type) {
    ondas_wlf_sv_enum table;
    if (!sv_table(type, (int)d->out.width, d->size, &table)) return 0;
    d->variants = calloc(table.count, sizeof(*d->variants));
    uint8_t *bits = malloc(d->out.width);
    if (!d->variants || !bits) { free(bits); return -1; }
    d->enum_count = table.count;
    for (size_t i = 0; i < d->enum_count; ++i) {
        ondas_wlf_enum_entry v;
        memcpy(&v, (const uint8_t *)table.entries + i * sizeof(v), sizeof(v));
        if (!v.label || !v.value) { free(bits); clear_enum(d); return 0; }
        sv_bits(v.value, (int)d->out.width, sv_storage((int)d->out.width, d->size), bits);
        if (add_variant(d, i, bits, v.label)) { free(bits); return -1; }
    }
    free(bits); return 0;
}
static int read_ordinal_enum(declaration *d, char **labels, int count) {
    d->variants = calloc((size_t)count, sizeof(*d->variants));
    if (!d->variants) return -1;
    d->enum_count = (size_t)count;
    WlfValueId display = wlfValueCreate(d->symbol);
    if (!display) { clear_enum(d); return 0; }
    int result = 0;
    for (uint32_t i = 0; i < (uint32_t)count; ++i) {
        /* VHDL stores enumeration positions, not SV assigned integers.
         * Verify each literal against the native typed value before export. */
        void *raw = wlfValueGetValue(display);
        if (!raw || !labels[i]) { clear_enum(d); break; }
        memcpy(raw, &i, (size_t)d->size);
        const char *label = wlfValueToString(display, WLF_RADIX_SYMBOLIC, INT_MAX);
        if (!label || strcmp(label, labels[i])) { clear_enum(d); break; }
        uint8_t bits[32];
        for (uint32_t bit = 0; bit < d->out.width; ++bit)
            bits[bit] = (uint8_t)('0' + ((i >> (d->out.width - 1 - bit)) & 1));
        if (add_variant(d, i, bits, labels[i])) { result = -1; break; }
    }
    wlfValueDestroy(display); return result;
}
static int add_decl(ondas_wlf *r, WlfSymbolId symbol, size_t parent, int scope, const char *name, size_t base_len, int array_verilog) {
    if (r->count == r->capacity) {
        size_t capacity = r->capacity ? r->capacity * 2 : 64;
        if (capacity < r->capacity || capacity > SIZE_MAX / sizeof(declaration)) return -1;
        declaration *decls = realloc(r->decls, capacity * sizeof(*decls));
        if (!decls) return -1;
        r->decls = decls; r->capacity = capacity;
    }
    declaration *d = &r->decls[r->count++];
    memset(d, 0, sizeof(*d));
    d->symbol = symbol; d->out.parent = parent; d->out.entry = scope ? 0 : 1;
    d->out.id = -1;
    d->out.name = copy(name ? name : wlfSymPropString(symbol, WLF_PROP_SYMBOL_NAME));
    WlfSymbolSel64 kind = wlfSymPropSymbolSel64(symbol, WLF_PROP_SYMBOL_TYPE);
    d->out.verilog_name = (kind & wlfSelVlog) != 0 || array_verilog;
    if (base_len > UINT32_MAX) return -1;
    d->out.array_base_len = (uint32_t)base_len;
    const char *canonical = scope ? scope_kind(kind) : variable_kind(kind);
    char fallback[64]; snprintf(fallback, sizeof(fallback), "wlf:%" PRIu64, (uint64_t)kind);
    d->out.kind = copy(canonical ? canonical : fallback);
    if (!d->out.name || !*d->out.name || !d->out.kind) return -1;
    if (scope) {
        if (kind & wlfSelModule) {
            const char *name = wlfSymPropString(symbol, WLF_PROP_PRIMARY_NAME);
            if (name && *name) { d->out.definition = copy(name); if (!d->out.definition) return -1; }
        }
        return 0;
    }
    WlfTypeId type = wlfSymPropTypeId(symbol, WLF_PROP_TYPE_ID);
    d->datatype = wlfTypePropDataType(type, WLF_TYPE_TYPE);
    d->size = wlfTypePropInt(type, WLF_TYPE_VALUE_SIZE);
    int width = wlfTypePropInt(type, WLF_TYPE_REGISTER_WIDTH);
    int left = wlfTypePropInt(type, WLF_TYPE_ARRAY_LEFT), right = wlfTypePropInt(type, WLF_TYPE_ARRAY_RIGHT);
    WlfTypeId element = type;
    if (d->datatype == wlfTypeArray) {
        width = wlfTypePropInt(type, WLF_TYPE_ARRAY_LENGTH);
        element = wlfTypePropTypeId(type, WLF_TYPE_ARRAY_ELEMENT_TYPE_ID);
    }
    unsigned element_type = element ? wlfTypePropDataType(element, WLF_TYPE_TYPE) : wlfTypeUnknown;
    if ((d->datatype == wlfTypeReg || d->datatype == wlfTypeNet || d->datatype == wlfTypeBit || d->datatype == wlfTypeByte) && width > 0) {
        d->out.encoding = 1;
        d->out.signedness = wlfTypePropInt(type, WLF_TYPE_VLOG_SIGNED) ? 2 : 1;
        d->out.logic_domain = d->datatype == wlfTypeBit ? 1 : 2;
    } else if (d->datatype == wlfTypeInteger && d->size == 4) {
        d->out.encoding = 1; width = 32; d->out.signedness = 2; d->out.logic_domain = 1;
    } else if ((d->datatype == wlfTypeReal || d->datatype == wlfTypeVlogReal) && d->size == 8) {
        d->out.encoding = 2;
    } else if (d->datatype == wlfTypeEvent) {
        d->out.encoding = 4;
    } else if (d->datatype == wlfTypeString && d->size == 8) {
        d->out.encoding = 3;
    } else if (d->datatype == wlfTypeVlogEnum && sv_storage(width, d->size)) {
        d->out.encoding = 1;
        d->out.signedness = wlfTypePropInt(type, WLF_TYPE_VLOG_SIGNED) ? 2 : 1;
        d->out.logic_domain = (size_t)d->size == (((size_t)width + 31) / 32) * 4 ? 1 : 2;
    } else if (element_type == wlfTypeEnum) {
        char **literals = NULL; int count = 0;
        if (wlfEnumLiterals(element, &literals, &count) != WLF_OK) return -1;
        const char *logic[] = {"'U'", "'X'", "'0'", "'1'", "'Z'", "'W'", "'L'", "'H'", "'-'"};
        int match = count == 9 || count == 2;
        for (int i = 0; match && i < count; ++i)
            match = literals[i] && !strcmp(literals[i], count == 9 ? logic[i] : logic[i+2]);
        if (match) {
            d->out.encoding = 1; d->out.logic_domain = count == 9 ? 3 : 1;
            if (d->datatype == wlfTypeEnum) width = 1;
            if (d->datatype == wlfTypeEnum) {
                d->out.width = 1;
                d->variants = calloc((size_t)count, sizeof(*d->variants));
                if (!d->variants) return -1;
                d->enum_count = (size_t)count;
                const uint8_t *keys = (const uint8_t *)(count == 9 ? "ux01zwlh-" : "01");
                for (int i = 0; i < count; ++i)
                    if (add_variant(d, (size_t)i, &keys[i], literals[i])) return -1;
            }
        } else if (d->datatype == wlfTypeEnum && count > 0 && left == 0 && right == count - 1 &&
                   (d->size == 1 || d->size == 2 || d->size == 4) &&
                   (uint64_t)(unsigned)count <= ((uint64_t)1 << (d->size * 8))) {
            d->out.encoding = 1; width = d->size * 8;
            d->out.width = (uint32_t)width;
            if (read_ordinal_enum(d, literals, count)) return -1;
        }
    }
    if (d->out.encoding == 1 && width > 0) {
        d->out.width = (uint32_t)width;
        if (d->datatype != wlfTypeEnum && (width > 1 || left != 0 || right != 0) && llabs((long long)left - right) + 1 == width) {
            d->out.has_range = 1; d->out.msb = left; d->out.lsb = right;
        }
    } else if (d->out.encoding == 1) d->out.encoding = 0;
    if (d->datatype == wlfTypeVlogEnum && d->out.encoding == 1 && read_sv_enum(d, type)) return -1;
    d->out.is_constant = !!(kind & (wlfSelParameter | wlfSelConstant | wlfSelGeneric | wlfSelSpecparam));
    if (wlfSymIsModeSelect(symbol, wlfModeSelInputs)) d->out.direction = 1;
    else if (wlfSymIsModeSelect(symbol, wlfModeSelOutputs)) d->out.direction = 2;
    else if (wlfSymIsModeSelect(symbol, wlfModeSelInouts)) d->out.direction = 3;
    WlfSymbolId actual = symbol;
    size_t hops = 0;
    for (;;) {
        WlfSymbolId alias = wlfSymPropSymbolId(actual, WLF_PROP_ALIAS_SYMBOL_ID);
        if (!alias) break;
        if (++hops > 1024) return -1;
        actual = alias;
    }
    int archive = wlfSymPropInt(actual, WLF_PROP_ARCHIVE_NUMBER);
    if (archive < 0) archive = wlfSymPropInt(symbol, WLF_PROP_ARCHIVE_NUMBER);
    d->out.id = archive;
    return 0;
}
static int walk(ondas_wlf *r, WlfSymbolId symbol, size_t parent, unsigned depth, int root, const char *name, size_t base_len, int array_verilog) {
    if (depth > 1024) return -1;
    WlfTypeId type = wlfSymPropTypeId(symbol, WLF_PROP_TYPE_ID);
    int scope = root || wlfSymIsSymbolSelect64(symbol, wlfSelAllScopes) || !type;
    if (!root) {
        size_t index = r->count;
        if (add_decl(r, symbol, parent, scope, name, base_len, array_verilog)) return -1;
        if (!scope) {
            if (wlfTypePropDataType(type, WLF_TYPE_TYPE) != wlfTypeArray || r->decls[index].out.encoding)
                return 0;
            /* A container has several archives, not one scalar history. Keep
             * its declaration, and expose SDK elements as exact components. */
            r->decls[index].out.id = -1;
            WlfIterId elements = wlfSymSubelements(symbol);
            if (!elements) return 0;
            const char *prefix = r->decls[index].out.name;
            size_t source_len = base_len ? base_len : strlen(prefix);
            int source_verilog = r->decls[index].out.verilog_name;
            WlfSymbolId child; int result = 0;
            /* IteratorCount covers child-scope iterators, not subelements. */
            int expected = wlfSymCountSubelements(symbol), observed = 0;
            while ((child = wlfIterate(elements))) {
                ++observed;
                const char *suffix = wlfSymPropString(child, WLF_PROP_SYMBOL_NAME);
                if (!suffix || strlen(prefix) > SIZE_MAX - strlen(suffix) - 1) { result = -1; break; }
                char *component = malloc(strlen(prefix) + strlen(suffix) + 1);
                if (!component) { result = -1; break; }
                strcpy(component, prefix); strcat(component, suffix);
                result = walk(r, child, parent, depth + 1, 0, component, source_len, source_verilog);
                free(component);
                if (result) break;
            }
            if (wlfIteratorDestroy(elements) != WLF_OK || expected != observed) result = -1;
            return result;
        }
        parent = index;
    }
    WlfIterId iterator = wlfSymChildren64(symbol, ~(WlfSymbolSel64)0);
    if (!iterator) return 0;
    int expected = wlfIteratorCount(iterator), observed = 0, result = 0;
    WlfSymbolId child;
    while ((child = wlfIterate(iterator))) {
        ++observed;
        if (walk(r, child, parent, depth+1, 0, NULL, 0, 0)) { result = -1; break; }
    }
    if (wlfIteratorDestroy(iterator) != WLF_OK || expected != observed) result = -1;
    return result;
}
int ondas_wlf_open(const char *path, const char *name, int metadata_only, ondas_wlf **out, char *error, size_t cap) {
    *out = NULL;
    if (!initialized) {
        if (wlfInit() != WLF_OK) return fail(error, cap, -3, wlfErrorMsg());
        initialized = 1;
    }
    ondas_wlf *r = calloc(1, sizeof(*r));
    if (!r) return fail(error, cap, -1, "cannot allocate WLF reader");
    if (namespace_guard) setCurrentNamespace(namespace_guard);
    current_expr_contxt = NULL; current_expr_objtd = NULL;
    if (wlfFileHeader((char *)path, &r->info) != WLF_OK) {
        int code = sdk_error(error, cap); free(r); return code;
    }
    if (r->info.startTime < 0 || r->info.lastTime < r->info.startTime || r->info.resolutionLimit < 0 || r->info.resolutionLimit >= 18) {
        free(r); return fail(error, cap, -2, "invalid WLF time bounds or resolution");
    }
    if (!metadata_only) {
        current_expr_contxt = NULL; current_expr_objtd = NULL;
        /* Keep one lite owner alive across closes. Both lite and normal open
         * consult the process namespace before replacing it. */
        if (next_guard == UINT64_MAX) { free(r); return fail(error, cap, -1, "WLF namespace identifier overflow"); }
        char guard_name[64];
        snprintf(guard_name, sizeof(guard_name), "ondas-guard-%" PRIu64, ++next_guard);
        WlfFileId guard = wlfFileOpenByMode((char *)path, guard_name, WLF_LITE_OPEN);
        if (!guard) {
            int malformed = wlfErrorNum() == 4 && r->info.magic == 0x1364;
            int code = fail(error, cap, malformed ? -2 : -1, wlfErrorMsg());
            free(r); return code;
        }
        setCurrentNamespace(guard);
        if (namespace_guard) wlfFileClose(namespace_guard);
        namespace_guard = guard;
        current_expr_contxt = NULL; current_expr_objtd = NULL;
        r->file = wlfFileOpen((char *)path, (char *)name);
        int open_code = 0;
        if (!r->file) {
            /* Unclosed snapshots can report error 4 with this header marker.
               Error 4 alone also describes a locked file; retain Backend there. */
            int malformed = wlfErrorNum() == 4 && r->info.magic == 0x1364;
            open_code = fail(error, cap, malformed ? -2 : -1, wlfErrorMsg());
        } else setCurrentNamespace(r->file);
        if (!r->file) { free(r); return open_code; }
        WlfSymbolId root = wlfFileGetTopRegion(r->file);
        if (!root || walk(r, root, SIZE_MAX, 0, 1, NULL, 0, 0)) {
            ondas_wlf_close(r); return fail(error, cap, -1, "cannot read WLF declarations");
        }
    }
    *out = r; return 0;
}
void ondas_wlf_metadata(ondas_wlf *r, ondas_wlf_meta *out) {
    *out = (ondas_wlf_meta){(uint64_t)r->info.startTime, (uint64_t)r->info.lastTime, (uint32_t)r->info.resolutionLimit, r->info.productName};
}
size_t ondas_wlf_decl_count(ondas_wlf *r) { return r->count; }
void ondas_wlf_declaration(ondas_wlf *r, size_t index, ondas_wlf_decl *out) { *out = r->decls[index].out; }
size_t ondas_wlf_enum_count(ondas_wlf *r, size_t index) { return r->decls[index].enum_count; }
void ondas_wlf_enum_variant(ondas_wlf *r, size_t index, size_t variant, const char **bits, const char **label) {
    *bits = r->decls[index].variants[variant].bits;
    *label = r->decls[index].variants[variant].label;
}
void ondas_wlf_end(ondas_wlf *r) {
    if (r->file) setCurrentNamespace(r->file);
    if (r->pack) { wlfPackDestroy(r->pack); r->pack = NULL; }
    for (size_t i = 0; i < r->selected_count; ++i) {
        if (r->selected[i].value) wlfValueDestroy(r->selected[i].value);
        free(r->selected[i].data);
    }
    free(r->selected); r->selected = NULL; r->selected_count = 0;
    free(r->records); r->records = NULL; r->record_count = r->record_capacity = 0;
    free(r->record_bytes); r->record_bytes = NULL;
    r->record_bytes_len = r->record_bytes_capacity = 0;
}
void ondas_wlf_close(ondas_wlf *r) {
    if (!r) return;
    ondas_wlf_end(r);
    if (r->file) {
        current_expr_contxt = NULL; current_expr_objtd = NULL;
        setCurrentNamespace(namespace_guard);
        wlfFileClose(r->file);
    }
    for (size_t i = 0; i < r->count; ++i) {
        free((void *)r->decls[i].out.name); free((void *)r->decls[i].out.kind); free((void *)r->decls[i].out.definition);
        clear_enum(&r->decls[i]);
    }
    free(r->decls); free(r);
}
void ondas_wlf_resources(ondas_wlf *r, size_t *count, size_t *bytes, int *pack) {
    *count = r->selected_count; *bytes = 0; *pack = r->pack != NULL;
    for (size_t i = 0; i < r->selected_count; ++i) *bytes += r->selected[i].len;
    *bytes += r->record_bytes_capacity + r->record_capacity * sizeof(*r->records);
}
static WlfCallbackResponse callback_failure(ondas_wlf *r, const char *message) {
    snprintf(r->failure, sizeof(r->failure), "%s", message);
    return WLF_ABORT_SCAN;
}
static const uint8_t logic_digits[256] = {
    ['0'] = '0', ['1'] = '1', ['-'] = '-',
    ['x'] = 'x', ['X'] = 'x', ['z'] = 'z', ['Z'] = 'z',
    ['u'] = 'u', ['U'] = 'u', ['w'] = 'w', ['W'] = 'w',
    ['l'] = 'l', ['L'] = 'l', ['h'] = 'h', ['H'] = 'h',
};
static WlfCallbackResponse store_value(selected *s) {
    ondas_wlf *r = s->owner;
    const void *value = wlfValueGetValue(s->value);
    if (!value) return callback_failure(r, "missing WLF value buffer");
    if (s->decl->out.encoding == 3) {
        /* For native SV strings this API returns the literal contents,
           without quotes/escapes; qualified with the authored byte probes. */
        const char *text = wlfValueToString(s->value, WLF_RADIX_SYMBOLIC, INT_MAX);
        if (!text) return callback_failure(r, "missing WLF string value");
        size_t len = strlen(text);
        if (len != s->len) {
            uint8_t *bytes = len ? realloc(s->data, len) : NULL;
            if (len && !bytes) return callback_failure(r, "cannot allocate WLF string value");
            if (!len) free(s->data);
            s->data = bytes; s->len = len;
        }
        if (len) memcpy(s->data, text, len);
    } else if (s->decl->out.encoding == 2) memcpy(&s->real_bits, value, 8);
    else if (s->decl->datatype == wlfTypeVlogEnum)
        sv_bits(value, (int)s->len, sv_storage((int)s->len, s->decl->size), s->data);
    else if (s->decl->datatype == wlfTypeEnum && !s->decl->out.logic_domain) {
        uint32_t ordinal = 0; memcpy(&ordinal, value, (size_t)s->decl->size);
        for (size_t i = 0; i < s->len; ++i)
            s->data[i] = (uint8_t)('0' + ((ordinal >> (s->len - 1 - i)) & 1));
    }
    else if (s->decl->datatype == wlfTypeInteger) {
        uint32_t bits; memcpy(&bits, value, 4);
        for (unsigned i = 0; i < 32; ++i) s->data[i] = (uint8_t)('0' + ((bits >> (31-i)) & 1));
    } else {
        const char *bits = wlfValueToString(s->value, WLF_RADIX_BINARY, (int)s->len + 128);
        if (!bits || strlen(bits) != s->len) return callback_failure(r, "invalid WLF bit width");
        if (!s->present || memcmp(s->data, bits, s->len) != 0) {
            for (size_t i = 0; i < s->len; ++i) {
                uint8_t digit = logic_digits[(unsigned char)bits[i]];
                if (!digit) return callback_failure(r, "invalid WLF logic digit");
                s->data[i] = digit;
            }
        }
    }
    s->present = 1;
    return WLF_CONTINUE_SCAN;
}
static WlfCallbackResponse change(void *data, WlfCallbackReason reason) {
    selected *s = data; ondas_wlf *r = s->owner;
    if (reason == WLF_ENDLOG) return WLF_CONTINUE_SCAN;
    if (reason != WLF_STARTLOG && reason != WLF_EVENT) return callback_failure(r, "unsupported WLF callback reason");
    WlfTime64 tick;
    if (wlfPackTime(r->pack, &tick) != WLF_OK || tick < r->start || tick > r->end) return callback_failure(r, "invalid WLF callback time");
    if (r->have_tick && tick != r->tick) return callback_failure(r, "incomplete WLF tick boundary");
    r->tick = tick; r->have_tick = 1;
    if (s->decl->out.encoding == 4) {
        if (reason == WLF_EVENT) {
            if (s->occurrences == UINT64_MAX) return callback_failure(r, "WLF event count overflow");
            ++s->occurrences; s->present = 1;
        }
    } else {
        return store_value(s);
    }
    return WLF_CONTINUE_SCAN;
}
static void *reserve_buffer(void *data, size_t *capacity, size_t needed, size_t element) {
    if (needed <= *capacity) return data;
    size_t next = *capacity <= SIZE_MAX / 2 ? *capacity * 2 : *capacity;
    if (next < needed) next = needed;
    if (next > SIZE_MAX / element) return NULL;
    void *value = realloc(data, next * element);
    if (!value) return NULL;
    *capacity = next;
    return value;
}
/* Copy only completed ticks. Soft limits permit one larger complete tick. */
static int complete_tick(ondas_wlf *r) {
    if (!r->have_tick) return 1;
    size_t count = r->record_count, bytes = r->record_bytes_len;
    for (size_t i = 0; i < r->selected_count; ++i) {
        selected *s = &r->selected[i];
        if (!s->present) continue;
        if (count == SIZE_MAX || s->len > SIZE_MAX - bytes) return 0;
        ++count; bytes += s->len;
    }
    if (count > r->record_capacity) {
        buffered_record *records = reserve_buffer(r->records, &r->record_capacity, count, sizeof(*r->records));
        if (!records) return 0;
        r->records = records;
    }
    if (bytes > r->record_bytes_capacity) {
        uint8_t *data = reserve_buffer(r->record_bytes, &r->record_bytes_capacity, bytes, 1);
        if (!data) return 0;
        r->record_bytes = data;
    }
    for (size_t i = 0; i < r->selected_count; ++i) {
        selected *s = &r->selected[i];
        if (!s->present) continue;
        buffered_record *record = &r->records[r->record_count++];
        record->out = (ondas_wlf_record){i, (uint64_t)r->tick, s->occurrences, NULL, s->len, s->real_bits};
        record->offset = r->record_bytes_len;
        if (s->len) memcpy(r->record_bytes + r->record_bytes_len, s->data, s->len);
        r->record_bytes_len += s->len;
        s->present = 0; s->occurrences = 0;
    }
    r->have_tick = 0;
    return 1;
}
static WlfCallbackResponse before_tick(void *data, WlfTime64 tick, int delta) {
    (void)delta;
    ondas_wlf *r = data;
    if (r->have_tick && tick > r->tick) {
        if (!complete_tick(r)) return callback_failure(r, "cannot buffer completed WLF tick");
        if (r->record_count >= WLF_BATCH_RECORDS || r->record_bytes_len >= WLF_BATCH_BYTES) {
            r->next_tick = tick; r->stopped = 1; return WLF_STOP_SCAN;
        }
    }
    return WLF_CONTINUE_SCAN;
}
int ondas_wlf_begin(ondas_wlf *r, const size_t *indices, size_t count, uint64_t start, uint64_t end, char *error, size_t cap) {
    ondas_wlf_end(r);
    r->start = start > (uint64_t)r->info.lastTime ? r->info.lastTime : (start > (uint64_t)r->info.startTime ? (WlfTime64)start : r->info.startTime);
    r->end = end < (uint64_t)r->info.lastTime ? (WlfTime64)end : r->info.lastTime;
    r->finished = start > (uint64_t)r->info.lastTime || r->end < r->start || !count;
    r->failure[0] = 0; r->cursor = 0; r->have_tick = 0;
    if (r->finished) return 0;
    r->selected = calloc(count, sizeof(*r->selected));
    if (!r->selected) return fail(error, cap, -1, "cannot allocate WLF selection");
    r->selected_count = count;
    r->pack = wlfPackCreate();
    if (!r->pack) return sdk_error(error, cap);
    for (size_t i = 0; i < count; ++i) {
        if (indices[i] >= r->count) return fail(error, cap, -1, "invalid WLF declaration index");
        selected *s = &r->selected[i]; s->owner = r; s->decl = &r->decls[indices[i]];
        if (!s->decl->out.encoding || s->decl->out.id < 0) return fail(error, cap, -1, "unsupported WLF selection");
        if (s->decl->out.encoding == 1) {
            s->len = s->decl->out.width;
            if (s->len > INT_MAX - 128) return fail(error, cap, -1, "WLF bit width exceeds formatter limit");
            s->data = malloc(s->len);
            if (!s->data) return fail(error, cap, -1, "cannot allocate WLF bit value");
        }
        s->value = wlfValueCreate(s->decl->symbol);
        if (!s->value || wlfAddSignalEventCB(r->pack, s->decl->symbol, s->value, WLF_REQUEST_IMMEDIATE, change, s) != WLF_OK) return sdk_error(error, cap);
    }
    if (wlfAddPreTimeDeltaCB(r->pack, before_tick, NULL, r) != WLF_OK) return sdk_error(error, cap);
    return 0;
}
int ondas_wlf_next(ondas_wlf *r, ondas_wlf_record *out, char *error, size_t cap) {
    setCurrentNamespace(r->file);
    for (;;) {
        if (r->cursor < r->record_count) {
            buffered_record *record = &r->records[r->cursor++];
            *out = record->out;
            out->data = out->len ? r->record_bytes + record->offset : NULL;
            return 1;
        }
        if (*r->failure) return fail(error, cap, -1, r->failure);
        if (r->finished) return 0;
        r->record_count = 0; r->record_bytes_len = 0; r->cursor = 0;
        r->have_tick = 0; r->stopped = 0;
        int code = wlfReadDataOverRange(r->pack, r->start, r->start == r->info.startTime ? r->info.startDelta : 0, r->end, WLF_LAST_DELTA, NULL, NULL, NULL);
        if (code != WLF_OK && !*r->failure) {
            const char *message = wlfErrorMsg();
            fail(r->failure, sizeof(r->failure), -1, message && *message ? message : "WLF SDK read failed");
        }
        if (!*r->failure && !complete_tick(r)) snprintf(r->failure, sizeof(r->failure), "cannot buffer completed WLF tick");
        if (r->stopped) {
            if (r->next_tick <= r->start) return fail(error, cap, -1, "WLF range made no progress");
            r->start = r->next_tick;
        } else r->finished = 1;
    }
}
int ondas_wlf_batch_end(ondas_wlf *r) {
    if (r->cursor < r->record_count || *r->failure) return 0;
    return r->finished ? 2 : 1;
}
