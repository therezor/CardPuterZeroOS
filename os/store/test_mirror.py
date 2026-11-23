"""Tests for mirror.py. Run: python3 -m unittest discover os/store

The golden files in testdata/ are what the fixture registry turns into.
gh0st-core's store tests parse the same files, so the two sides can't
drift. UPDATE=1 writes them again.
"""

import json
import os
import shutil
import subprocess
import tempfile
import unittest

import mirror

HERE = os.path.dirname(os.path.abspath(__file__))
DATA = os.path.join(HERE, "testdata")


def row(sha, pkg, **kw):
    r = {c: "" for c in mirror.CHECKED_COLS}
    r.update(sha256=sha, package=pkg, version=kw.pop("version", "1.0"), arch="arm64",
             scripts="0", paths="0", service="0")
    r.update(kw)
    return r


def fixture():
    with open(os.path.join(DATA, "registry.json"), encoding="utf-8") as f:
        apps = json.load(f)["apps"]
    h = lambda c: c * 64
    checked = {
        h("a"): row(h("a"), "acidzero", version="0.2.0", depends="libc6, libsdl2-2.0-0 | libsdl2",
                    desktop="acidzero"),
        h("b"): row(h("b"), "rootsetup", scripts="1", paths="1", service="1", depends="python3:any"),
        h("c"): row(h("c"), "needsfoo", depends="libc6, foo (>= 2)"),
        h("d"): row(h("d"), "bash", version="9.9"),
        h("f"): row(h("f"), "takeover", replaces="coreutils"),
    }
    names = {"libc6", "libsdl2-2.0-0", "python3", "bash", "coreutils"}
    cn = {"acidzero": {"sha256": h("a"),
                       "url": "https://cardputer-zero-repo.oss-cn-shenzhen.aliyuncs.com/packages/pool/main/acidzero/acidzero_0.2.0_arm64.deb"},
          "rootsetup": {"sha256": h("0"), "url": "https://cardputer-zero-repo.oss-cn-shenzhen.aliyuncs.com/packages/x.deb"}}
    return apps, checked, names, cn


class Golden(unittest.TestCase):
    def test_the_fixture_makes_the_golden_files(self):
        apps, checked, names, cn = fixture()
        dropped = {}
        for a in apps:
            why = mirror.usable(a)
            if why:
                dropped[a["download"]["package"]] = why
        lines, kept = mirror.listing(apps, checked, names, {"gh0st"}, cn, dropped)
        shots = {}
        for a in kept:
            for i, u in enumerate(a["screenshots"]):
                shots[u] = {"url": u, "etag": "", "sha256": str(i) * 64, "ok": "1"}
        about = "".join(sorted(mirror.about_line(a, shots) for a in kept))
        pin = (mirror.hashlib.sha256(about.encode()).hexdigest(), len(about.encode()))
        catalog = mirror.catalog_text(1763870000, 1766462000, pin, ("1" * 64, 20), sorted(lines))
        out = {"catalog": catalog, "about": about,
               "dropped": "".join(f"{p}\t{w}\n" for p, w in sorted(dropped.items()))}
        for name, text in out.items():
            path = os.path.join(DATA, name)
            if os.environ.get("UPDATE"):
                with open(path, "w", encoding="utf-8") as f:
                    f.write(text)
            with open(path, encoding="utf-8") as f:
                self.assertEqual(f.read(), text, name)


class Text(unittest.TestCase):
    def test_clean_drops_control_characters_and_caps(self):
        self.assertEqual(mirror.clean("a\tb\r\nc\x1b[31m d", 40), "a b c [31m d")
        self.assertEqual(mirror.clean("abcdef", 4), "abc…")

    def test_plain_keeps_paragraphs_and_drops_markdown(self):
        s = mirror.plain("**Bold** [link](https://x)\r\n\r\n## Head\nline\\nmore")
        # Paragraphs join with a backslash-n; a literal one loses its slash.
        self.assertEqual(s, "Bold link\\nHead linenmore")

    def test_urls_must_be_https_on_known_hosts(self):
        self.assertTrue(mirror.url_ok("https://github.com/CardputerZero/packages/releases/download/a/b.deb", mirror.DEB_HOSTS))
        for bad in ["http://github.com/CardputerZero/packages/releases/download/a.deb",
                    "https://github.com/someone/packages/releases/download/a.deb",
                    "https://github.com/CardputerZero/packages/releases/download/a b.deb",
                    None]:
            self.assertFalse(mirror.url_ok(bad, mirror.DEB_HOSTS), bad)


    def test_english_comes_from_the_en_locale_when_the_top_is_cjk(self):
        app = {"summary": "\u6d77\u6218", "title": "Battleship",
               "locales": {"en": {"summary": "Naval combat"}}}
        self.assertEqual(mirror.english(app, "summary"), "Naval combat")
        self.assertEqual(mirror.english(app, "title"), "Battleship")
        self.assertEqual(mirror.english({"summary": "\u6d77"}, "summary"), "\u6d77")


class Depends(unittest.TestCase):
    def test_a_group_needs_one_offered_name(self):
        names = {"libc6", "python3", "libsdl2"}
        self.assertIsNone(mirror.resolvable("libc6 (>= 2.36), python3:any, foo | libsdl2", names))
        self.assertEqual(mirror.resolvable("libc6, rpicam-apps (>= 1)", names), "rpicam-apps (>= 1)")
        self.assertIsNone(mirror.resolvable("", names))

    def test_an_index_offers_its_packages_and_provides(self):
        idx = "Package: a\nProvides: b (= 1), c:any\n\nPackage: d\n"
        self.assertEqual(mirror.index_names(idx), {"a", "b", "c", "d"})


class Icons(unittest.TestCase):
    def test_a_pack_reads_back(self):
        icons = {"b": b"\x89PNG two", "a": b"\x89PNG one"}
        pack = mirror.build_pack(icons)
        self.assertTrue(pack.startswith(b"gh0st-icons 1\na 0 8\nb 8 8\n\n"))
        self.assertEqual(mirror.read_pack(pack), icons)
        self.assertEqual(mirror.read_pack(b"junk"), {})


@unittest.skipUnless(shutil.which("dpkg-deb"), "needs dpkg-deb")
class Debs(unittest.TestCase):
    def test_facts_see_scripts_paths_units_and_the_desktop_file(self):
        with tempfile.TemporaryDirectory() as d:
            root = os.path.join(d, "root")
            for p in ["DEBIAN", "usr/bin", "usr/share/APPLaunch/applications", "etc/systemd/system"]:
                os.makedirs(os.path.join(root, p))
            with open(os.path.join(root, "DEBIAN/control"), "w") as f:
                f.write("Package: demo\nVersion: 1.2-1\nArchitecture: arm64\nMaintainer: x <x@example.invalid>\n"
                        "Depends: libc6\nDescription: demo\n")
            with open(os.path.join(root, "DEBIAN/postinst"), "w") as f:
                f.write("#!/bin/sh\n")
            os.chmod(os.path.join(root, "DEBIAN/postinst"), 0o755)
            for p in ["usr/bin/demo", "usr/share/APPLaunch/applications/demo.desktop",
                      "etc/systemd/system/demo.service"]:
                open(os.path.join(root, p), "w").close()
            deb = os.path.join(d, "demo.deb")
            subprocess.run(["dpkg-deb", "--root-owner-group", "-b", root, deb],
                           check=True, capture_output=True)
            f = mirror.facts(deb, "0" * 64)
            self.assertEqual((f["package"], f["version"], f["arch"]), ("demo", "1.2-1", "arm64"))
            self.assertEqual((f["scripts"], f["paths"], f["service"]), ("1", "1", "1"))
            self.assertEqual((f["depends"], f["desktop"]), ("libc6", "demo"))


if __name__ == "__main__":
    unittest.main()
