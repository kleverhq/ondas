#ifndef ONDAS_VWDB_ABI_H
#define ONDAS_VWDB_ABI_H
#include <cstdint>

namespace vwdb {
struct Object;
using Native = Object *;
struct Stamp { std::uint64_t tick, sequence; };
struct DString { char *string; int length, space; char storage[200]; };
static_assert(sizeof(Stamp) == 16 && sizeof(DString) == 216, "VWDB ABI layout");

enum Property {
    Error = 0x12, EndOptions = 0x10, ErrorMessage = 0x1a,
    IsConstant = 0x122, Name = 0x10a, RawValue = 0x10b, ScopeKind = 0x113,
    FirstIndex = 0x11b, Load = 0x12f, Time = 0x136, Unload = 0x13f,
    FormattedValue = 0x15a, RawBits = 0x164, TypeKind = 0x173, NextValue = 0x17f,
    ElementCount = 0x184, HasRange = 0x192, Ascending = 0x1a2, State = 0x1a3,
    SeekBound = 0x126, PackedLayout = 0x1b2, ValueFormat = 0x1b3, Type = 0x1be, IndexType = 0x1de,
    EnumElements = 0x1fe, FormatOptions = 0x204, History = 0x22e,
    Scopes = 0x2ae, Variables = 0x2be, AllowSummary = 0x302, Width = 0x3d4,
};
enum ScopeType { ArchitectureScope = 3, ModuleScope = 6, ArrayScope = 10 };
enum TypeKind { Event = 1, Scalar, Enum, Vector, Real, Character = 7, Array, Record };

constexpr int binary_format = 1;
constexpr int full_width = 0x8000;
constexpr int tcl_dont_wait = 1 << 1;
constexpr int tcl_timer_events = 1 << 4;
constexpr int tcl_idle_events = 1 << 5;

// Keep every private entry point local to the selected DSO.
struct Abi {
    void (*init)();
    int (*open)(const char *, Native *, int (*)(void *, int), void *);
    int (*close)(Native);
    int (*precision)(unsigned *);
    int (*first)(Native, Stamp *);
    int (*last)(Native, Stamp *);
    int (*get)(int, Native);
    int (*get_int)(int, Native, int *);
    int (*get_bytes)(int, Native, DString *);
    const char *(*get_text)(int, Native);
    const char *(*format)(int, Native, ...);
    int (*get_stamp)(int, Native, Stamp *);
    Native (*handle)(int, Native);
    Native (*configure)(int, Native, ...);
    Native (*iterate)(int, Native);
    Native (*scan)(Native);
    void (*drop)(Native);
    void (*string_init)(DString *);
    void (*string_free)(DString *);
    int (*process_event)(int);
};
}
#endif
