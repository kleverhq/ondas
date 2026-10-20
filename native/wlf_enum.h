/* Private, qualified Linux x86-64 SDK views. No SDK object leaves the shim. */
#ifndef ONDAS_WLF_ENUM_H
#define ONDAS_WLF_ENUM_H

/* The SDK scalar descriptor ends with its kind-specific payload. The public
 * property getters do not expose the SV enum entry list. Qualification must
 * check this view against independent tables and the native typed formatter. */
typedef struct {
    unsigned char scalar_properties[56];
    const void *payload;
} ondas_wlf_type_view;
typedef struct {
    const char *label;
    const void *value;
} ondas_wlf_enum_entry;
typedef struct {
    const void *entries;
    WlfTypeId base, owner;
    unsigned count;
} ondas_wlf_sv_enum;
_Static_assert(offsetof(ondas_wlf_type_view, payload) == 56, "SV enum descriptor ABI mismatch");
_Static_assert(sizeof(ondas_wlf_enum_entry) == 16 && offsetof(ondas_wlf_sv_enum, count) == 24,
               "SV enum table ABI mismatch");

static int sv_storage(int width, int size) {
    if (width <= 0) return 0;
    size_t words = ((size_t)width + 31) / 32;
    return (size_t)size == words * 4 ? 1 : (size_t)size == words * 8 ? 2 : 0;
}
/* Native SV storage interleaves value and unknown words for four-state types.
 * Use the same conversion for declarations and samples; ignore word padding. */
static void sv_bits(const void *raw, int width, int storage, uint8_t *out) {
    const uint8_t *bytes = raw;
    for (int bit = width - 1; bit >= 0; --bit) {
        unsigned word = (unsigned)bit / 32, shift = (unsigned)bit % 32;
        uint32_t value, unknown = 0;
        memcpy(&value, bytes + (size_t)word * (size_t)storage * 4, 4);
        if (storage == 2) memcpy(&unknown, bytes + (size_t)word * 8 + 4, 4);
        out[width - 1 - bit] = (uint8_t)"01zx"[(((unknown >> shift) & 1) << 1) | ((value >> shift) & 1)];
    }
}

/* Return an absent view when its scalar properties do not agree. This is an
 * ABI qualification check, not a portable decoder of arbitrary SDK layouts. */
static int sv_table(WlfTypeId type, int width, int size, ondas_wlf_sv_enum *table) {
    ondas_wlf_type_view view;
    memcpy(&view, type, sizeof(view));
    if (!view.payload) return 0;
    memcpy(table, view.payload, sizeof(*table));
    if (table->owner != type || !table->base || !table->entries || !table->count)
        return 0;
    if (wlfTypePropInt(table->base, WLF_TYPE_REGISTER_WIDTH) != width ||
        wlfTypePropInt(table->base, WLF_TYPE_VALUE_SIZE) != size)
        return 0;
    return 1;
}
#endif
