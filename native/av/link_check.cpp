// The link's path escaping against the exact text gh0st_core::link writes
// (its any_path_survives_a_field test), and a round trip.
#include "link.h"

#include <cstdio>

using gh0st::pathField;
using gh0st::pathOf;

static int fails = 0;

static void expect(bool ok, const char *what)
{
    printf("%s %s\n", ok ? "ok  " : "FAIL", what);
    fails += !ok;
}

int main()
{
    const std::string odd = "/home/u/a\tb\nc%d \xff\xc3\xa9.txt";
    expect(pathField(odd) == "/home/u/a%09b%0Ac%25d %FF\xc3\xa9.txt", "the same text as Rust");
    expect(pathOf(pathField(odd)) == odd, "any path round trips");
    expect(pathOf("/plain/path") == "/plain/path", "a plain path is itself");
    expect(pathOf("/bad%2") == "/bad%2" && pathOf("/end%") == "/end%", "a broken escape stays");
    expect(pathField("\xed\xa0\x80") == "%ED%A0%80", "a surrogate is not UTF-8");
    expect(pathField("\xf0\x9f\x8e\xb5") == "\xf0\x9f\x8e\xb5", "four bytes stay");
    return fails ? 1 : 0;
}
