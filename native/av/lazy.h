// Libraries a helper loads only when a file needs them: the tempo filter,
// subtitles, HEIC and RAW. Linked, Debian's libavfilter alone maps speech
// voices, libplacebo and more into every helper that never changes speed.
// The headers still declare each function; GH0ST_LAZY makes lz_<name>, a
// call through the library opened at first use. Call lazy::open first: a
// function of a library that isn't there is never called.
#pragma once

namespace gh0st::lazy {

inline constexpr const char *AVFILTER = "libavfilter.so.10";
inline constexpr const char *ASS = "libass.so.9";
inline constexpr const char *HEIF = "libheif.so.1";
inline constexpr const char *RAW = "libraw.so.23";

// Opens `soname` once. False when it isn't installed.
bool open(const char *soname);
// The address of `name` in `soname`, opened already. Null if missing.
void *sym(const char *soname, const char *name);

} // namespace gh0st::lazy

#define GH0ST_LAZY(soname, name)                                                    \
    template <typename... A> inline auto lz_##name(A... a)                          \
    {                                                                               \
        using F = decltype(&::name);                                                \
        static F f = reinterpret_cast<F>(gh0st::lazy::sym(soname, #name));          \
        return f(a...);                                                             \
    }
