#ifndef ONDAS_SHM_SST2_H
#define ONDAS_SHM_SST2_H
#include <cstdint>

// Recovered, private x86_64 ABI. Only libsst2 owns these handles. Alias identity
// is optional; readers without it keep declarations separate.
using SstHandle = std::uintptr_t;
struct SstTime { std::uint64_t tick, sequence; };
static_assert(sizeof(SstTime) == 16, "SST2 ABI layout");
#ifdef ONDAS_SST2_INIT_CPP
extern int sst2Init();
#else
extern "C" int sst2Init();
#endif
extern "C" {
int drOpenFile(const char *, SstHandle *, int (*)(void *, int), void *);
int drClose(SstHandle);
int da_get(int, SstHandle);
int da_get_int(int, SstHandle, int *);
int da_get_real(int, SstHandle, double *);
const char *da_get_sstr(int, SstHandle);
const char *da_get_sstr_pl(int, SstHandle, ...);
#ifdef ONDAS_SHM_USE_ALIAS_IDENTITY
SstHandle da_get_xref(int, SstHandle);
#endif
int da_get_xvalseq(int, SstHandle, SstTime *);
SstHandle da_handle(int, SstHandle);
SstHandle da_handle_pl(int, SstHandle, ...);
SstHandle da_iterate(int, SstHandle);
SstHandle da_scan(SstHandle);
void da_delete(SstHandle);
}
#endif
