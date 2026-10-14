#include "shm_value.h"
#include <cassert>
#include <cstdio>

int main() {
    using ondas_shm_value::decode_quoted;
    assert(decode_quoted("\"\"").empty());
    assert(decode_quoted("\"a\\\"b\\\\c\\n\\t\"") == "a\"b\\c\n\t");
    assert(decode_quoted("\"\\\\xFF\"") == "\\xFF");
    assert(decode_quoted("\"A\\x00B\"") == std::string("A\0B", 3));
    assert(decode_quoted("\"\\xC0\\x80\"") == std::string("\xc0\x80", 2));
    assert(decode_quoted("\"\\xF0\\x9F\\x8C\\x8A\"") == "\xf0\x9f\x8c\x8a");
    assert(decode_quoted("\"\\xffF\"") == std::string("\xff" "F", 2));
    assert(decode_quoted(("\"" + std::string(6144, 'A') + "\"").c_str()) == std::string(6144, 'A'));
    std::string encoded = "\"", expected;
    for (int byte = 0; byte <= 255; ++byte) {
        char hex[5];
        std::snprintf(hex, sizeof(hex), "\\x%02X", byte);
        encoded += hex;
        expected.push_back(static_cast<char>(byte));
    }
    encoded += '"';
    assert(decode_quoted(encoded.c_str()) == expected);
    const char *invalid_inputs[] = {nullptr, "", "x", "\"", "\"x", "x\"", "\"\\\"",
                                   "\"\\x\"", "\"\\x1\"", "\"\\xGG\"", "\"\\q\"", "\"x\"y\"",
                                   "\"\n\"", "\"\xff\""};
    for (const char *invalid : invalid_inputs) {
        bool failed = false;
        try { (void) decode_quoted(invalid); }
        catch (const std::runtime_error &) { failed = true; }
        assert(failed);
    }
}
