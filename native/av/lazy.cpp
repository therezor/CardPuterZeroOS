#include "lazy.h"

#include <dlfcn.h>
#include <map>
#include <mutex>
#include <string>

namespace gh0st::lazy {

namespace {
std::mutex m;
std::map<std::string, void *> libs;
} // namespace

bool open(const char *soname)
{
    std::lock_guard<std::mutex> g(m);
    auto it = libs.find(soname);
    if (it != libs.end())
        return it->second != nullptr;
    void *h = dlopen(soname, RTLD_NOW | RTLD_LOCAL);
    libs[soname] = h;
    return h != nullptr;
}

void *sym(const char *soname, const char *name)
{
    if (!open(soname))
        return nullptr;
    std::lock_guard<std::mutex> g(m);
    return dlsym(libs[soname], name);
}

} // namespace gh0st::lazy
