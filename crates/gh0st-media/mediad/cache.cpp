#include "cache.h"

#include <algorithm>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <ctime>
#include <dirent.h>
#include <sys/stat.h>
#include <unistd.h>
#include <vector>

namespace mediad {

namespace {

const char MAGIC[4] = {'G', '0', 'T', 'H'};
// Bump when tiles are drawn differently, so old ones are not reused.
const uint32_t VERSION = 1;
// A tile seen again is marked new at most this often.
const time_t TOUCH_SECONDS = 24 * 3600;

uint64_t fnv(uint64_t h, const void *p, size_t n)
{
    const uint8_t *b = static_cast<const uint8_t *>(p);
    for (size_t i = 0; i < n; i++) {
        h ^= b[i];
        h *= 1099511628211ull;
    }
    return h;
}

} // namespace

ThumbCache::ThumbCache(std::string dir) : dir_(std::move(dir))
{
    if (dir_.empty())
        return;
    // Each level, as mkdir -p.
    for (size_t at = 1; at != std::string::npos; at = dir_.find('/', at + 1))
        mkdir(dir_.substr(0, at).c_str(), 0700);
    mkdir(dir_.c_str(), 0700);
}

std::string ThumbCache::defaultDir()
{
    const char *x = getenv("XDG_CACHE_HOME");
    std::string base = x && *x ? x : std::string(getenv("HOME") ? getenv("HOME") : "/tmp") + "/.cache";
    return base + "/gh0st-media/thumbs";
}

std::string ThumbCache::name(const std::string &path, int w, int h) const
{
    struct stat st;
    if (dir_.empty() || stat(path.c_str(), &st) != 0)
        return "";
    uint64_t k = 14695981039346656037ull;
    k = fnv(k, path.data(), path.size());
    int64_t parts[] = {int64_t(st.st_mtim.tv_sec), int64_t(st.st_mtim.tv_nsec), int64_t(st.st_size),
                       w, h, VERSION};
    k = fnv(k, parts, sizeof parts);
    char buf[24];
    snprintf(buf, sizeof buf, "%016llx", (unsigned long long)k);
    return dir_ + "/" + buf + ".565";
}

bool ThumbCache::get(const std::string &path, int w, int h, gh0st::Pic &out, bool &bad) const
{
    bad = false;
    std::string n = name(path, w, h);
    FILE *f = n.empty() ? nullptr : fopen(n.c_str(), "rb");
    if (!f)
        return false;
    uint8_t head[16];
    size_t got = fread(head, 1, sizeof head, f);
    if (got == 0) {
        fclose(f);
        bad = true;
        return false;
    }
    uint16_t tw, th;
    memcpy(&tw, head + 4, 2);
    memcpy(&th, head + 6, 2);
    bool ok = got == sizeof head && memcmp(head, MAGIC, 4) == 0 && tw && th && tw <= w && th <= h;
    if (ok) {
        out.w = tw;
        out.h = th;
        out.px.resize(size_t(tw) * th);
        ok = fread(out.px.data(), 2, out.px.size(), f) == out.px.size();
    }
    struct stat st;
    if (ok && fstat(fileno(f), &st) == 0 && st.st_mtime < time(nullptr) - TOUCH_SECONDS)
        futimens(fileno(f), nullptr);
    fclose(f);
    return ok;
}

void ThumbCache::put(const std::string &path, int w, int h, const gh0st::Pic *pic)
{
    std::string n = name(path, w, h);
    if (n.empty())
        return;
    std::string part = n + ".part";
    FILE *f = fopen(part.c_str(), "wb");
    if (!f)
        return;
    bool ok = true;
    uint8_t head[16] = {};
    if (pic) {
        memcpy(head, MAGIC, 4);
        uint16_t tw = uint16_t(pic->w), th = uint16_t(pic->h);
        memcpy(head + 4, &tw, 2);
        memcpy(head + 6, &th, 2);
        ok = fwrite(head, 1, sizeof head, f) == sizeof head &&
             fwrite(pic->px.data(), 2, pic->px.size(), f) == pic->px.size();
    }
    ok = fclose(f) == 0 && ok;
    if (!ok || rename(part.c_str(), n.c_str()) != 0) {
        unlink(part.c_str());
        return;
    }
    // A tile written over another counts twice until the next trim, which
    // counts the folder afresh.
    total_ += pic ? sizeof head + pic->px.size() * 2 : 0;
    if (most_ && total_ > most_)
        trim(most_, keep_);
}

void ThumbCache::trim(size_t most, size_t keep)
{
    most_ = most;
    keep_ = keep;
    DIR *d = dir_.empty() ? nullptr : opendir(dir_.c_str());
    if (!d)
        return;
    struct Entry {
        std::string path;
        time_t when;
        size_t size;
    };
    std::vector<Entry> all;
    size_t total = 0;
    while (dirent *e = readdir(d)) {
        if (e->d_name[0] == '.')
            continue;
        std::string p = dir_ + "/" + e->d_name;
        struct stat st;
        if (stat(p.c_str(), &st) != 0 || !S_ISREG(st.st_mode))
            continue;
        all.push_back({p, st.st_mtime, size_t(st.st_size)});
        total += size_t(st.st_size);
    }
    closedir(d);
    total_ = total;
    if (total <= most)
        return;
    std::sort(all.begin(), all.end(), [](const Entry &a, const Entry &b) { return a.when < b.when; });
    for (auto &e : all) {
        if (total <= keep)
            break;
        if (unlink(e.path.c_str()) == 0)
            total -= e.size;
    }
    total_ = total;
}

} // namespace mediad
