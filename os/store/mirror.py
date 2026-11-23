#!/usr/bin/env python3
"""Builds the Store's catalog from M5's registry (docs/store.md).

    mirror.py OUT [--apt DIR] [--keyring FILE] [--only PKG,...]
              [--registry FILE --cn-registry FILE]

OUT gets catalog, catalog.sig, about-<sha256>, icons-<sha256>, and the
files the next run starts from: checked.tsv (what each deb holds),
sources.tsv (icons and screenshots by ETag) and dropped.tsv (left out, and
why).

Environment:
  STORE_PREV  URL (or file:// URL) of the live store site. Unset: start empty.
  REPO_KEY    The armored secret signing key. Unset: GNUPGHOME holds it.

M5's registry is untrusted input. Every URL must be https on a known host,
every text field is cleaned and capped, every deb is checked against the
registry's SHA-256 before dpkg-deb reads it, and nothing is extracted. A
failure anywhere carries the previous site over unchanged. It exits 1
only when it can't do that either, since deploying without the store
would delete it from the site.
"""

import concurrent.futures
import hashlib
import io
import json
import lzma
import os
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
import time
import urllib.error
import urllib.request

REGISTRY = "https://raw.githubusercontent.com/CardputerZero/CardputerZero.github.io/main/generated/registry.json"
CN_REGISTRY = "https://cardputer-zero-repo.oss-cn-shenzhen.aliyuncs.com/packages/cn/registry.json"
DEBIAN = [
    "https://deb.debian.org/debian/dists/trixie/main/binary-arm64/Packages.xz",
    "https://deb.debian.org/debian/dists/trixie-updates/main/binary-arm64/Packages.xz",
    "https://deb.debian.org/debian-security/dists/trixie-security/main/binary-arm64/Packages.xz",
]
# Where each kind of URL may point.
DEB_HOSTS = ("https://github.com/CardputerZero/packages/releases/download/",)
CN_HOSTS = ("https://cardputer-zero-repo.oss-cn-shenzhen.aliyuncs.com/packages/",)
ART_HOSTS = ("https://raw.githubusercontent.com/CardputerZero/",)

MAGIC = "gh0st-store 1"
DAY = 86400
VALID_DAYS = 30
# A catalog with this much life left is served again as it is, so an
# unchanged list keeps its bytes and devices download nothing.
RESIGN_DAYS = 20
ICON_PX = 48
SHOT_W, SHOT_H = 320, 170
MAX_SHOT = 2_000_000
MAX_ICON = 2_000_000
MAX_JSON = 16_000_000
MAX_SHOTS = 6
CAPS = {"title": 40, "summary": 120, "author": 40, "licence": 40, "category": 30}
DESCRIPTION_CAP = 2000
# A run that finds far fewer apps than the last one is a broken source.
MIN_SHARE = 0.5

SCRIPTS = {"preinst", "postinst", "prerm", "postrm", "config"}
UNIT_DIRS = (
    "lib/systemd/",
    "usr/lib/systemd/",
    "etc/systemd/",
    "etc/init.d/",
)
CHECKED_COLS = [
    "sha256", "package", "version", "arch", "depends", "provides",
    "replaces", "conflicts", "breaks", "scripts", "paths", "service", "desktop",
]

URL_RE = re.compile(r"^https://[A-Za-z0-9._~:/?#@!$&'()*+,;=%-]+$")
PACKAGE_RE = re.compile(r"^[a-z0-9][a-z0-9+.-]{1,127}$")
VERSION_RE = re.compile(r"^[0-9][A-Za-z0-9.+~:-]{0,99}$")
SHA_RE = re.compile(r"^[0-9a-f]{64}$")
SHARE_RE = re.compile(r"^[A-Z0-9]{1,8}$")
DESKTOP_RE = re.compile(r"^[A-Za-z0-9._+-]{1,100}$")


def log(msg):
    print(f"== {msg}", flush=True)


def warn(msg):
    print(f"::warning::store: {msg}", flush=True)


# ---------------------------------------------------------------- fetching


class HttpsOnly(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        if not newurl.startswith("https://"):
            raise urllib.error.URLError(f"redirect to {newurl} refused")
        return super().redirect_request(req, fp, code, msg, headers, newurl)


OPENER = urllib.request.build_opener(HttpsOnly)


def fetch(url, cap, etag=None):
    """(status, bytes, etag). 304 gives no bytes. file:// reads a file."""
    if url.startswith("file://"):
        with open(url[7:], "rb") as f:
            return 200, f.read(cap + 1)[:cap], None
    if not url.startswith("https://"):
        raise ValueError(f"not https: {url}")
    req = urllib.request.Request(url, headers={"User-Agent": "gh0st-store-mirror"})
    if etag:
        req.add_header("If-None-Match", etag)
    for attempt in range(3):
        try:
            with OPENER.open(req, timeout=60) as r:
                data = r.read(cap + 1)
                if len(data) > cap:
                    raise ValueError(f"{url}: over {cap} bytes")
                return r.status, data, r.headers.get("ETag")
        except urllib.error.HTTPError as e:
            if e.code == 304:
                return 304, b"", etag
            if e.code == 404 or attempt == 2:
                raise
        except (urllib.error.URLError, TimeoutError):
            if attempt == 2:
                raise
        time.sleep(2 * (attempt + 1))
    raise RuntimeError("unreachable")


def fetch_prev(base, name, cap=MAX_JSON):
    """A file of the previous site, or None when it isn't there. Any
    other failure raises: a flaky fetch must not read as "empty"."""
    if not base:
        return None
    try:
        return fetch(f"{base.rstrip('/')}/{name}", cap)[1]
    except urllib.error.HTTPError as e:
        if e.code == 404:
            return None
        raise
    except FileNotFoundError:
        return None


def download(url, sha256, size, path):
    """Streams a deb to path and checks it before anything reads it.
    Three tries: one dropped connection shouldn't drop an app for a day."""
    for attempt in range(3):
        try:
            return download_once(url, sha256, size, path)
        except (urllib.error.URLError, TimeoutError, ConnectionError):
            if attempt == 2:
                raise
            time.sleep(5 * (attempt + 1))


def download_once(url, sha256, size, path):
    if not url.startswith("https://"):
        raise ValueError(f"not https: {url}")
    h = hashlib.sha256()
    n = 0
    req = urllib.request.Request(url, headers={"User-Agent": "gh0st-store-mirror"})
    with OPENER.open(req, timeout=120) as r, open(path, "wb") as f:
        while True:
            chunk = r.read(1 << 20)
            if not chunk:
                break
            n += len(chunk)
            if n > size:
                raise ValueError(f"{url}: bigger than the registry says")
            h.update(chunk)
            f.write(chunk)
    if n != size or h.hexdigest() != sha256:
        raise ValueError(f"{url}: size or SHA-256 differs from the registry")


# ---------------------------------------------------------------- text


def clean(s, cap):
    """One field: no control characters, no tab or newline, capped."""
    s = "".join(" " if ord(c) < 32 or ord(c) == 127 or c in "  \x85" else c for c in str(s or ""))
    s = re.sub(r"\s+", " ", s).strip()
    if len(s) > cap:
        s = s[: cap - 1].rstrip() + "…"
    return s


def plain(md):
    """M5's descriptions carry some markdown. Plain text, paragraphs kept."""
    s = str(md or "").replace("\r\n", "\n").replace("\r", "\n")
    s = re.sub(r"!\[[^\]]*\]\([^)]*\)", "", s)
    s = re.sub(r"\[([^\]]*)\]\([^)]*\)", r"\1", s)
    s = re.sub(r"^[ \t]{0,3}#{1,6}[ \t]*", "", s, flags=re.M)
    # A list item gets a paragraph of its own.
    s = re.sub(r"^[ \t]*[-*+][ \t]+", "\n• ", s, flags=re.M)
    s = s.replace("**", "").replace("__", "").replace("`", "")
    # A literal backslash-n would read as a new paragraph on the device.
    s = re.sub(r"\\(?=n)", "", s)
    paras = [clean(p, DESCRIPTION_CAP) for p in re.split(r"\n\s*\n", s)]
    out = "\\n".join(p for p in paras if p)
    if len(out) > DESCRIPTION_CAP:
        out = out[: DESCRIPTION_CAP - 1].rstrip("\\ ") + "…"
    return out


CJK = re.compile(r"[\u3000-\u9fff\uac00-\ud7af\uff00-\uffef]")


def english(app, key):
    """A field in English: the top level unless it is CJK, then M5's en
    locale when it has one. The device has no CJK font."""
    top = app.get(key) or ""
    if not CJK.search(str(top)):
        return top
    for src in (app.get("locales"), app.get("i18n")):
        for lang in ("en", "en-US"):
            v = ((src or {}).get(lang) or {}).get(key) if isinstance(src, dict) else None
            if isinstance(v, str) and v and not CJK.search(v):
                return v
    return top


def url_ok(url, hosts):
    return isinstance(url, str) and URL_RE.match(url) and url.startswith(hosts)


# ---------------------------------------------------------------- debs


def control_fields(deb):
    names = ["Package", "Version", "Architecture", "Depends", "Pre-Depends",
             "Provides", "Replaces", "Conflicts", "Breaks"]
    out = subprocess.run(["dpkg-deb", "-f", "--", deb] + names,
                         check=True, capture_output=True, text=True).stdout
    fields, key = {}, None
    for line in out.splitlines():
        if line[:1] in (" ", "\t") and key:
            fields[key] += " " + line.strip()
        elif ":" in line:
            key, _, v = line.partition(":")
            fields[key] = v.strip()
    return fields


def members(deb, part):
    """The tar members of a deb's control or data part, listed only."""
    p = subprocess.Popen(["dpkg-deb", f"--{part}-tarfile", "--", deb], stdout=subprocess.PIPE)
    try:
        with tarfile.open(fileobj=p.stdout, mode="r|") as t:
            return [(m.name, m.isdir(), m.mode) for m in t]
    finally:
        p.stdout.close()
        if p.wait() != 0:
            raise ValueError(f"{deb}: dpkg-deb --{part}-tarfile failed")


def facts(deb, sha256):
    """What a deb holds, for checked.tsv."""
    f = control_fields(deb)
    ctrl = {os.path.basename(n) for n, _, _ in members(deb, "ctrl")}
    data = members(deb, "fsys")
    pkg = f.get("Package", "")
    own = (f"usr/lib/{pkg}/", f"usr/share/{pkg}/", f"usr/share/doc/{pkg}/",
           "usr/bin/", "usr/share/APPLaunch/")
    paths = service = False
    desktop = ""
    for name, is_dir, mode in data:
        n = name[2:] if name.startswith("./") else name.lstrip("/")
        if is_dir or not n:
            continue
        if not n.startswith(own) or mode & 0o6000:
            paths = True
        if n.startswith(UNIT_DIRS):
            service = True
        m = re.fullmatch(r"usr/share/APPLaunch/applications/([^/]+)\.desktop", n)
        if m and not desktop:
            desktop = m.group(1)
    depends = ", ".join(x for x in (f.get("Pre-Depends", ""), f.get("Depends", "")) if x)
    return {
        "sha256": sha256,
        "package": pkg,
        "version": f.get("Version", ""),
        "arch": f.get("Architecture", ""),
        "depends": depends,
        "provides": f.get("Provides", ""),
        "replaces": f.get("Replaces", ""),
        "conflicts": f.get("Conflicts", ""),
        "breaks": f.get("Breaks", ""),
        "scripts": "1" if ctrl & SCRIPTS else "0",
        "paths": "1" if paths else "0",
        "service": "1" if service else "0",
        "desktop": desktop,
    }


def read_tsv(data, cols):
    rows = {}
    for line in (data or b"").decode("utf-8", "replace").splitlines():
        f = line.split("\t")
        if len(f) == len(cols):
            rows[f[0]] = dict(zip(cols, f))
    return rows


def write_tsv(rows, cols):
    return "".join("\t".join(clean(r[c], 4000) for c in cols) + "\n"
                   for _, r in sorted(rows.items())).encode()


# ---------------------------------------------------------------- packages


def names_of(field):
    """Package names in a Depends-like field, without versions or arch."""
    out = []
    for part in re.split(r"[,|]", field or ""):
        n = re.sub(r"\(.*?\)|\[.*?\]|<.*?>", "", part).strip().split(":")[0].strip()
        if n:
            out.append(n)
    return out


def index_names(text):
    """Every package and virtual package an apt index offers."""
    names = set()
    for stanza in text.split("\n\n"):
        for line in stanza.splitlines():
            if line.startswith("Package:"):
                names.add(line[8:].strip())
            elif line.startswith("Provides:"):
                names.update(names_of(line[9:]))
    return names


def resolvable(depends, names):
    """The first Depends group nothing offers, or None."""
    for group in (depends or "").split(","):
        alts = names_of(group)
        if alts and not any(a in names for a in alts):
            return group.strip()
    return None


# ---------------------------------------------------------------- art


def png_size(data):
    if data[:8] != b"\x89PNG\r\n\x1a\n" or data[12:16] != b"IHDR":
        return None
    return int.from_bytes(data[16:20], "big"), int.from_bytes(data[20:24], "big")


def small_icon(data):
    """A 48 px PNG, decoded under a pixel cap and encoded again."""
    from PIL import Image  # CI only

    Image.MAX_IMAGE_PIXELS = 4096 * 4096
    size = png_size(data)
    if not size or size[0] * size[1] > Image.MAX_IMAGE_PIXELS:
        return None
    im = Image.open(io.BytesIO(data))
    im.load()
    im = im.convert("RGBA")
    im.thumbnail((ICON_PX, ICON_PX), Image.LANCZOS)
    out = io.BytesIO()
    im.save(out, "PNG", optimize=True)
    return out.getvalue()


def read_pack(data):
    if not data:
        return {}
    end = data.find(b"\n\n")
    head = data[:end].decode("utf-8", "replace").splitlines() if end > 0 else []
    if not head or head[0] != "gh0st-icons 1":
        return {}
    base = end + 2
    out = {}
    for line in head[1:]:
        f = line.split(" ")
        if len(f) == 3 and f[1].isdigit() and f[2].isdigit():
            o, n = int(f[1]), int(f[2])
            if base + o + n <= len(data):
                out[f[0]] = data[base + o: base + o + n]
    return out


def about_line(app, sources):
    shots = []
    for u in (app.get("screenshots") or [])[:MAX_SHOTS]:
        r = sources.get(u)
        if r and r["ok"] == "1":
            shots.append(f"{r['sha256']} {u}")
    pkg = app["download"]["package"]
    return "\t".join([pkg, plain(english(app, "description"))] + shots) + "\n"


def build_pack(icons):
    head = "gh0st-icons 1\n"
    body = b""
    for pkg in sorted(icons):
        head += f"{pkg} {len(body)} {len(icons[pkg])}\n"
        body += icons[pkg]
    return (head + "\n").encode() + body


# ---------------------------------------------------------------- catalog


def catalog_text(date, expires, about, icons, lines):
    return (f"{MAGIC}\ndate {date}\nexpires {expires}\n"
            f"about {about[0]} {about[1]}\nicons {icons[0]} {icons[1]}\n" + "".join(lines))


def body_of(text):
    """A catalog without its dates, to tell whether anything changed."""
    return "\n".join(l for l in text.splitlines() if not l.startswith(("date ", "expires ")))


def header(text, key):
    for line in text.splitlines():
        if line.startswith(key + " "):
            return line.split(" ")[1:]
    return None


def app_line(app, facts_row, cn):
    """The catalog line of one registry app, or (None, reason)."""
    d = app.get("download") or {}
    pkg = d.get("package", "")
    perms = app.get("permissions") or {}
    if d.get("type") != "deb":
        return None, "not a deb"
    if facts_row["package"] != pkg:
        return None, f"the deb is {facts_row['package']}"
    if facts_row["arch"] not in ("arm64", "all"):
        return None, f"architecture {facts_row['arch']}"
    version = facts_row["version"]
    if not VERSION_RE.match(version):
        return None, "bad version"
    flags = []
    if facts_row["scripts"] == "1":
        flags.append("scripts")
    if facts_row["paths"] == "1":
        flags.append("paths")
    if facts_row["service"] == "1" or perms.get("background_service"):
        flags.append("service")
    for key, word in [("network", "network"), ("camera", "camera"),
                      ("microphone", "microphone"), ("additional_hardware", "hardware"),
                      ("external_display", "display"), ("imu", "imu")]:
        if perms.get(key) is True:
            flags.append(word)
    cats = app.get("categories") or ["Other"]
    share = str(app.get("share_code") or "")
    desktop = facts_row["desktop"] if DESKTOP_RE.match(facts_row["desktop"] or "") else ""
    author = app.get("author")
    if isinstance(author, dict):
        author = author.get("display_name", "")
    cn_url = ""
    if cn and cn.get("sha256") == d["sha256"] and url_ok(cn.get("url"), CN_HOSTS):
        cn_url = cn["url"]
    fields = [
        pkg,
        version,
        str(int(d["size"])),
        d["sha256"],
        clean(cats[0], CAPS["category"]),
        ",".join(flags) or "-",
        clean(english(app, "title") or pkg, CAPS["title"]),
        clean(english(app, "summary"), CAPS["summary"]),
        clean(author, CAPS["author"]),
        clean(app.get("license"), CAPS["licence"]),
        d["url"],
        cn_url,
        share if SHARE_RE.match(share) else "",
        desktop,
    ]
    return "app\t" + "\t".join(fields) + "\n", None


def usable(app):
    """Whether a registry entry is well formed enough to look at."""
    d = app.get("download") if isinstance(app, dict) else None
    if not isinstance(d, dict):
        return "no download"
    if not PACKAGE_RE.match(str(d.get("package", ""))):
        return "bad package name"
    if not SHA_RE.match(str(d.get("sha256", ""))):
        return "no SHA-256"
    if not isinstance(d.get("size"), int) or not 0 < d["size"] < 1 << 31:
        return "bad size"
    if not url_ok(d.get("url"), DEB_HOSTS):
        return "deb URL not on M5's GitHub release"
    return None


def listing(apps, checked, names, ours, cn, dropped):
    """The catalog's app lines and the apps they came from. Adds to
    dropped what it leaves out, and why."""
    lines, kept = [], []
    offered = names | ours
    for a in apps:
        d = a.get("download") or {}
        pkg = d.get("package")
        if pkg in dropped or d.get("sha256") not in checked:
            dropped.setdefault(str(pkg)[:60], "couldn't check the deb")
            continue
        row = checked[d["sha256"]]
        clash = [n for f in ("provides", "replaces", "conflicts", "breaks")
                 for n in names_of(row[f]) if n in names or n in ours]
        if pkg in names or pkg in ours:
            dropped[pkg] = "named like a Debian or Gh0st OS package"
            continue
        if clash:
            dropped[pkg] = f"takes over {clash[0]}"
            continue
        missing = resolvable(row["depends"], offered)
        if missing:
            dropped[pkg] = f"needs {missing}"
            continue
        line, why = app_line(a, row, cn.get(pkg))
        if line and row["version"] != a.get("version"):
            warn(f"{pkg}: the registry says {a.get('version')}, the deb {row['version']}")
        if not line:
            dropped[pkg] = why
            continue
        lines.append(line)
        kept.append(a)
    return lines, kept


# ---------------------------------------------------------------- the run


def sign(path, keyring):
    """A detached signature, made as publish.sh signs, then checked by sqv."""
    gpg = ["gpg", "--batch", "--yes", "--quiet", "--pinentry-mode", "loopback", "--digest-algo", "SHA512"]
    subprocess.run(gpg + ["--detach-sign", "-o", path + ".sig", path], check=True)
    subprocess.run(["sqv", "--keyring", keyring, "--", path + ".sig", path],
                   check=True, stdout=subprocess.DEVNULL)


def build(out, prev, apt, keyring, only, registry, cn_registry, work):
    now = int(time.time())
    reg = json.loads(fetch(registry, MAX_JSON)[1])
    apps = [a for a in reg.get("apps", []) if isinstance(a, dict)]
    try:
        cn = {(a.get("download") or {}).get("package"): a.get("download") or {}
              for a in json.loads(fetch(cn_registry, MAX_JSON)[1]).get("apps", [])}
    except Exception as e:  # the China mirror is optional
        warn(f"China registry: {e}")
        cn = {}
    if only:
        apps = [a for a in apps if (a.get("download") or {}).get("package") in only]

    names = set()
    for url in DEBIAN:
        names |= index_names(lzma.decompress(fetch(url, 64_000_000)[1]).decode("utf-8", "replace"))
    ours = set()
    for suite in ("edge", "stable"):
        p = os.path.join(apt or "", "dists", suite, "main", "binary-arm64", "Packages")
        if apt and os.path.exists(p):
            with open(p, encoding="utf-8", errors="replace") as f:
                ours |= index_names(f.read())
    if apt and not ours:
        raise RuntimeError(f"no Packages index under {apt}")
    log(f"{len(apps)} apps; {len(names)} Debian names, {len(ours)} of ours")

    checked = read_tsv(fetch_prev(prev, "checked.tsv"), CHECKED_COLS)
    dropped = {}
    seen = set()
    todo = []
    for a in apps:
        why = usable(a)
        pkg = (a.get("download") or {}).get("package", "?") if isinstance(a, dict) else "?"
        if why:
            dropped[str(pkg)[:60]] = why
        elif pkg in seen:
            dropped[pkg] = "listed twice"
        else:
            seen.add(pkg)
            if a["download"]["sha256"] not in checked:
                todo.append(a)
    log(f"{len(todo)} debs to check")

    def check(a):
        d = a["download"]
        path = os.path.join(work, d["sha256"] + ".deb")
        try:
            download(d["url"], d["sha256"], d["size"], path)
            return d["sha256"], facts(path, d["sha256"]), None
        except Exception as e:
            return d["sha256"], None, str(e)
        finally:
            if os.path.exists(path):
                os.remove(path)

    with concurrent.futures.ThreadPoolExecutor(4) as pool:
        for sha, row, err in pool.map(check, todo):
            if row:
                checked[sha] = row
            else:
                warn(err)

    lines, kept = listing(apps, checked, names, ours, cn, dropped)
    log(f"{len(lines)} listed, {len(dropped)} left out")

    prev_cat = fetch_prev(prev, "catalog")
    prev_text = prev_cat.decode("utf-8", "replace") if prev_cat else ""
    prev_n = prev_text.count("\napp\t")
    if not only and prev_n and len(lines) < prev_n * MIN_SHARE:
        raise RuntimeError(f"{len(lines)} apps, down from {prev_n}")

    # Art: icons and screenshots, fetched again only when their ETag moves.
    src_cols = ["url", "etag", "sha256", "ok"]
    sources = read_tsv(fetch_prev(prev, "sources.tsv"), src_cols)
    old_icons_name = header(prev_text, "icons")
    old_icons = read_pack(fetch_prev(prev, f"icons-{old_icons_name[0]}") if old_icons_name else None)

    def art(url, kind, pkg):
        s = sources.get(url, {})
        # An icon's ETag counts only while the old pack still has it.
        etag = s.get("etag") if kind == "shot" or pkg in old_icons else None
        try:
            status, data, etag = fetch(url, MAX_ICON if kind == "icon" else MAX_SHOT, etag)
        except Exception as e:
            warn(f"{url}: {e}")
            return url, None, None
        if status == 304:
            return url, s, old_icons.get(pkg) if kind == "icon" else None
        if kind == "icon":
            try:
                small = small_icon(data)
            except Exception:
                small = None
            sha = hashlib.sha256(small).hexdigest() if small else ""
            return url, {"url": url, "etag": etag or "", "sha256": sha, "ok": "1" if small else "0"}, small
        ok = png_size(data) == (SHOT_W, SHOT_H)
        return url, {"url": url, "etag": etag or "", "sha256": hashlib.sha256(data).hexdigest(),
                     "ok": "1" if ok else "0"}, None

    jobs = []
    for a in kept:
        pkg = a["download"]["package"]
        if url_ok(a.get("icon"), ART_HOSTS):
            jobs.append((a["icon"], "icon", pkg))
        for u in (a.get("screenshots") or [])[:MAX_SHOTS]:
            if url_ok(u, ART_HOSTS):
                jobs.append((u, "shot", pkg))
    icons, new_sources = {}, {}
    with concurrent.futures.ThreadPoolExecutor(8) as pool:
        for (url, kind, pkg), (_, row, small) in zip(jobs, pool.map(lambda j: art(*j), jobs)):
            if not row:
                continue
            new_sources[url] = row
            if small and hashlib.sha256(small).hexdigest() == row["sha256"]:
                icons[pkg] = small

    about_lines = [about_line(a, new_sources) for a in kept]
    about = "".join(sorted(about_lines)).encode()
    pack = build_pack(icons)
    about_pin = (hashlib.sha256(about).hexdigest(), len(about))
    icons_pin = (hashlib.sha256(pack).hexdigest(), len(pack))

    os.makedirs(out, exist_ok=True)
    for name, data in [(f"about-{about_pin[0]}", about), (f"icons-{icons_pin[0]}", pack)]:
        with open(os.path.join(out, name), "wb") as f:
            f.write(data)
    # The previous pair stays a run, for devices that got the catalog
    # before a CDN edge got the new files.
    for key in ("about", "icons"):
        h = header(prev_text, key)
        if h and h[0] not in (about_pin[0], icons_pin[0]):
            data = fetch_prev(prev, f"{key}-{h[0]}")
            if data:
                with open(os.path.join(out, f"{key}-{h[0]}"), "wb") as f:
                    f.write(data)

    lines.sort()
    text = catalog_text(0, 0, about_pin, icons_pin, lines)
    cat = os.path.join(out, "catalog")
    prev_sig = fetch_prev(prev, "catalog.sig")
    prev_date = int((header(prev_text, "date") or ["0"])[0] or 0)
    prev_exp = int((header(prev_text, "expires") or ["0"])[0] or 0)
    if prev_sig and body_of(prev_text) == body_of(text) and prev_exp - now > RESIGN_DAYS * DAY:
        log("catalog unchanged")
        with open(cat, "wb") as f:
            f.write(prev_cat)
        with open(cat + ".sig", "wb") as f:
            f.write(prev_sig)
    else:
        date = max(now, prev_date + 1)
        text = catalog_text(date, date + VALID_DAYS * DAY, about_pin, icons_pin, lines)
        with open(cat, "w", encoding="utf-8") as f:
            f.write(text)
        sign(cat, keyring)
        log(f"catalog signed, {len(text)} bytes")

    with open(os.path.join(out, "checked.tsv"), "wb") as f:
        f.write(write_tsv(checked, CHECKED_COLS))
    with open(os.path.join(out, "sources.tsv"), "wb") as f:
        f.write(write_tsv(new_sources, src_cols))
    with open(os.path.join(out, "dropped.tsv"), "w", encoding="utf-8") as f:
        f.writelines(f"{p}\t{clean(w, 200)}\n" for p, w in sorted(dropped.items()))


def carry(out, prev):
    """The previous site as it was: what a failed run publishes. All of it
    or nothing: a catalog without its about and icons would mislead."""
    text = fetch_prev(prev, "catalog")
    if not text:
        warn("no previous store site to carry over")
        return True
    t = text.decode("utf-8", "replace")
    files = {"catalog": text}
    need = ["catalog.sig"] + [f"{k}-{header(t, k)[0]}" for k in ("about", "icons") if header(t, k)]
    for name in need:
        data = fetch_prev(prev, name)
        if data is None:
            warn(f"the live store site has no {name}")
            return False
        files[name] = data
    for name in ("checked.tsv", "sources.tsv", "dropped.tsv"):
        data = fetch_prev(prev, name)
        if data is not None:
            files[name] = data
    os.makedirs(out, exist_ok=True)
    for name, data in files.items():
        with open(os.path.join(out, name), "wb") as f:
            f.write(data)
    log("carried the previous store site over")
    return True


def main(argv):
    args = {"--apt": None, "--keyring": None, "--only": None,
            "--registry": REGISTRY, "--cn-registry": CN_REGISTRY}
    pos = []
    it = iter(argv)
    for a in it:
        if a in args:
            args[a] = next(it)
        else:
            pos.append(a)
    if len(pos) != 1 or not args["--keyring"]:
        print(__doc__, file=sys.stderr)
        return 2
    out = pos[0]
    prev = os.environ.get("STORE_PREV", "")
    only = set(args["--only"].split(",")) if args["--only"] else None
    tmp = tempfile.mkdtemp()
    try:
        if os.environ.get("REPO_KEY"):
            home = os.path.join(tmp, "gnupg")
            os.mkdir(home, 0o700)
            os.environ["GNUPGHOME"] = home
            subprocess.run(["gpg", "--batch", "--quiet", "--import"],
                           input=os.environ["REPO_KEY"].encode(), check=True)
        work = os.path.join(tmp, "debs")
        os.mkdir(work)
        if os.path.isdir(out):
            shutil.rmtree(out)
        build(out, prev, args["--apt"], args["--keyring"], only,
              args["--registry"], args["--cn-registry"], work)
    except Exception as e:
        warn(f"build failed: {e}")
        if os.path.isdir(out):
            shutil.rmtree(out)
        try:
            if not carry(out, prev):
                return 1
        except Exception as e2:
            # Publishing without the store would delete it from the site.
            print(f"store: can't carry the live site over either: {e2}", file=sys.stderr)
            if os.path.isdir(out):
                shutil.rmtree(out)
            return 1
    finally:
        shutil.rmtree(tmp, ignore_errors=True)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
