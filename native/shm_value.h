#ifndef ONDAS_SHM_VALUE_H
#define ONDAS_SHM_VALUE_H
#include <cstring>
#include <stdexcept>
#include <string>

namespace ondas_shm_value {
// SST2 radix 5, flags 0x200: ASCII quotes and byte escapes. This is a
// transport representation; decoded bytes still need Rust's UTF-8 check.
inline std::string decode_quoted(const char *text) {
    auto require = [](bool ok) {
        if (!ok) throw std::runtime_error("invalid SHM quoted string transport");
    };
    require(text != nullptr);
    const size_t size = std::strlen(text);
    require(size >= 2 && text[0] == '"' && text[size - 1] == '"');
    std::string result;
    for (size_t i = 1; i < size - 1; ++i) {
        unsigned char byte = text[i];
        if (byte != '\\') {
            require(byte >= 0x20 && byte <= 0x7e && byte != '"');
        } else {
            require(++i < size - 1);
            switch (text[i]) {
                case '"': byte = '"'; break;
                case '\\': byte = '\\'; break;
                case 'n': byte = '\n'; break;
                case 't': byte = '\t'; break;
                case 'x': {
                    require(i + 2 < size - 1);
                    auto hex = [](unsigned char digit) -> int {
                        if (digit >= '0' && digit <= '9') return digit - '0';
                        if (digit >= 'A' && digit <= 'F') return digit - 'A' + 10;
                        if (digit >= 'a' && digit <= 'f') return digit - 'a' + 10;
                        return -1;
                    };
                    const int high = hex(text[i + 1]), low = hex(text[i + 2]);
                    require(high >= 0 && low >= 0);
                    byte = static_cast<unsigned char>((high << 4) | low);
                    i += 2;
                    break;
                }
                default: require(false);
            }
        }
        result.push_back(static_cast<char>(byte));
    }
    return result;
}
}
#endif
