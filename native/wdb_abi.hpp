// Private reader ABI. Layout discovery and recorded-input qualification are separate checks.
#pragma once
#include <functional>
#include <string>
#include <vector>
#include <stdexcept>
static_assert(sizeof(std::string) == 32, "unsupported C++ string ABI");
template<class R, class... Args>
static R vcall(const void* obj, int slot, Args... args) {
    if (!obj) throw std::runtime_error("null native ABI object");
    auto table = *static_cast<void* const* const*>(obj);
    return reinterpret_cast<R(*)(const void*, Args...)>(table[slot])(obj, args...);
}

namespace SimBridge {
enum WdbOpenMode : int {};
struct IObject;
struct IType;
struct IDataObj;
struct IBlock;
struct Time { long long ticks; long long padding[3]{}; };
struct ValueFormat {
    int radix = 2;
    int radix_arg = 0;
    int width = 0;
    short precision = -12;
    unsigned char flags[10]{};
    std::string prefix;
    std::string suffix;
};
struct Value {
    Value() = default;
    Value(const Value&) = delete;
    Value& operator=(const Value&) = delete;
    int representation = 0;
    int kind = 0;
    union { unsigned long long integer; double real; char* text; } payload{};
    ~Value() { if (kind == 7) delete[] payload.text; }
};
struct IReader { enum TraverseCommand : int {}; };
class DataReader {
    // Inspected constructor writes a vptr and an implementation pointer (16 bytes).
    // Its deleting destructor passes size 0x10. This adapter uses the selected ABI only.
    alignas(16) unsigned char storage[64];
public:
    DataReader(const DataReader&) = delete;
    DataReader& operator=(const DataReader&) = delete;
    DataReader(WdbOpenMode, bool, const std::string&, const std::string&,
               const std::vector<std::string>&, int);
    ~DataReader();
    void* getDataObjMgr();
    int getPrecision() const;
    long long getLatestTime() const;
    const IObject* getRootObject() const;
    const IDataObj* getDataObj(const std::string&, int*) const;
    void traverseAllObjects(const std::function<IReader::TraverseCommand(const IObject*)>&,
                            bool, const IBlock*) const;
    long long getNextTransitionTime(const IDataObj*, long long) const;
    long long getPrevTransitionTime(const IDataObj*, long long) const;
    bool hasWaveform(const IDataObj*, const IDataObj**, bool*, std::string*) const;
    bool isTraceable(const IDataObj*, std::string*) const;
    void getValue(const IDataObj*, long long, const ValueFormat&, Value&) const;
    void setArrayDisplayLength(unsigned int);
};
}

namespace RTTI { struct FullConstraints; }
namespace SimBridge {
template<class T> struct BitIndex { T bytes; unsigned int bit = 0; };
class DataConversion {
public:
    void formatRawMemory(const BitIndex<const unsigned char*>&, const IType*,
                         const RTTI::FullConstraints*, const ValueFormat&, bool, Value&) const;
};
}
