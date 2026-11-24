// Host checks of the tile cache: a kept tile comes back, a failed file is
// remembered, a changed file misses, a broken tile is no tile, and trim
// drops the oldest tiles first down to its floor. `make check` runs it.
#include "cache.h"

#include <cstdio>
#include <cstdlib>
#include <dirent.h>
#include <fcntl.h>
#include <set>
#include <string>
#include <sys/stat.h>
#include <unistd.h>

using namespace mediad;

namespace {

int failed = 0;

void expect(bool ok, const char *what)
{
    std::printf("%s %s\n", ok ? "ok  " : "FAIL", what);
    failed += !ok;
}

std::set<std::string> files(const std::string &dir)
{
    std::set<std::string> all;
    if (DIR *d = opendir(dir.c_str())) {
        while (dirent *e = readdir(d))
            if (e->d_name[0] != '.')
                all.insert(dir + "/" + e->d_name);
        closedir(d);
    }
    return all;
}

// The file the last put added.
std::string added(const std::string &dir, std::set<std::string> &seen)
{
    for (auto &f : files(dir))
        if (seen.insert(f).second)
            return f;
    return "";
}

void age(const std::string &path, time_t when)
{
    timespec t[2] = {{when, 0}, {when, 0}};
    utimensat(AT_FDCWD, path.c_str(), t, 0);
}

void source(const std::string &path, const char *text)
{
    FILE *f = fopen(path.c_str(), "w");
    fputs(text, f);
    fclose(f);
}

gh0st::Pic tile(uint16_t colour)
{
    gh0st::Pic p;
    p.w = 10;
    p.h = 10;
    p.px.assign(100, colour);
    return p;
}

} // namespace

int main()
{
    char base[] = "/tmp/gh0st-cache-XXXXXX";
    if (!mkdtemp(base))
        return 1;
    std::string root = base, dir = root + "/thumbs";
    ThumbCache c(dir);
    std::string a = root + "/a.jpg", b = root + "/b.jpg", d = root + "/d.jpg", bad = root + "/bad.jpg";
    source(a, "a");
    source(b, "b");
    source(d, "d");
    source(bad, "x");

    std::set<std::string> seen;
    gh0st::Pic pa = tile(0xf800), pb = tile(0x07e0), pd = tile(0x001f);
    c.put(a, 80, 60, &pa);
    std::string ta = added(dir, seen);
    c.put(b, 80, 60, &pb);
    std::string tb = added(dir, seen);
    c.put(d, 80, 60, &pd);
    std::string td = added(dir, seen);
    c.put(bad, 80, 60, nullptr);
    std::string tbad = added(dir, seen);

    gh0st::Pic got;
    bool was_bad = true;
    expect(c.get(a, 80, 60, got, was_bad) && !was_bad && got.w == 10 && got.px == pa.px,
           "a kept tile comes back");
    expect(!c.get(a, 40, 30, got, was_bad), "another size misses");
    expect(!c.get(bad, 80, 60, got, was_bad) && was_bad, "a file that failed is remembered");

    source(b, "b, changed");
    expect(!c.get(b, 80, 60, got, was_bad) && !was_bad, "a changed file misses");

    FILE *f = fopen(td.c_str(), "r+b");
    fputs("JUNK", f);
    fclose(f);
    expect(!c.get(d, 80, 60, got, was_bad) && !was_bad, "a broken tile is no tile");

    // Tiles are 216 bytes, the marker none: 648 in all.
    age(ta, 1000);
    age(tb, 2000);
    age(td, 3000);
    age(tbad, 4000);
    c.trim(1000, 0);
    expect(files(dir).size() == 4, "under the cap, nothing goes");
    c.trim(500, 300);
    auto left = files(dir);
    expect(!left.count(ta) && !left.count(tb) && left.count(td) && left.count(tbad),
           "over it, the oldest go first down to the floor");

    ThumbCache none("");
    none.put(a, 80, 60, &pa);
    expect(!none.get(a, 80, 60, got, was_bad), "without a folder nothing is kept");

    std::string rm = "rm -rf '" + root + "'";
    if (system(rm.c_str()) != 0)
        return 1;
    return failed ? 1 : 0;
}
