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
             scripts="0", paths="0", service="0", facts=mirror.FACTS)
    r.update(kw)
    return r


def fixture():
    with open(os.path.join(DATA, "registry.json"), encoding="utf-8") as f:
        apps = json.load(f)["apps"]
    h = lambda c: c * 64
    checked = {
        h("a"): row(h("a"), "acidzero", version="0.2.0", depends="libc6, libsdl2-2.0-0 | libsdl2",
                    desktop="acidzero"),
        h("b"): row(h("b"), "rootsetup", scripts="1", paths="1", service="1", root="1",
                    depends="python3:any"),
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
        lines, kept = mirror.listing(apps, checked, names, {"gh0st"}, cn, dropped, None,
                                     {"a" * 64: ["zstd", "alsa-utils"]})
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

    def test_apts_reason_is_the_unmet_line(self):
        out = ("Reading package lists...\nSome packages could not be installed.\n"
               "The following packages have unmet dependencies:\n"
               " demo : Depends: python3-libcamera (>= 0.7) but 0.4-1 is to be installed\n"
               "E: Unable to correct problems, you have held broken packages.\n")
        self.assertEqual(mirror.apt_reason(out),
                         "Depends: python3-libcamera (>= 0.7) but 0.4-1 is to be installed")
        self.assertEqual(mirror.apt_reason("E: Unsupported file ./x.deb given on commandline\n"),
                         "Unsupported file ./x.deb given on commandline")

    def test_a_pinned_name_like_debians_is_an_app_pin_line(self):
        apps, checked, names, cn = fixture()
        names = names | {"acidzero"}
        dropped = {}
        lines, _ = mirror.listing(apps, checked, names, {"gh0st"}, cn, dropped)
        self.assertEqual(dropped["acidzero"], "named like a Debian or Gh0st OS package")
        old = mirror.PINNED
        mirror.PINNED = {"acidzero"}
        try:
            dropped = {}
            lines, _ = mirror.listing(apps, checked, names, {"gh0st"}, cn, dropped)
            self.assertNotIn("acidzero", dropped)
            self.assertTrue(any(l.startswith("app-pin\tacidzero\t") for l in lines))
            dropped = {}
            mirror.listing(apps, checked, names, {"gh0st", "acidzero"}, cn, dropped)
            self.assertIn("acidzero", dropped, "ours is never pinned out")
        finally:
            mirror.PINNED = old

    def test_an_app_apt_cant_install_is_left_out_with_apts_reason(self):
        apps, checked, names, cn = fixture()
        sha = "a" * 64
        why = "apt can't install it: Depends: udisks2 but it is not installable"
        dropped = {}
        lines, _ = mirror.listing(apps, checked, names, {"gh0st"}, cn, dropped, {sha: why})
        self.assertEqual(dropped["acidzero"], why)
        self.assertFalse(any("\tacidzero\t" in l for l in lines))


class Icons(unittest.TestCase):
    def test_a_pack_reads_back(self):
        icons = {"b": b"\x89PNG two", "a": b"\x89PNG one"}
        pack = mirror.build_pack(icons)
        self.assertTrue(pack.startswith(b"gh0st-icons 1\na 0 8\nb 8 8\n\n"))
        self.assertEqual(mirror.read_pack(pack), icons)
        self.assertEqual(mirror.read_pack(b"junk"), {})


class Tmpfiles(unittest.TestCase):
    def test_tmpfiles_stay_in_the_apps_own_folders(self):
        self.assertTrue(mirror.own_tmpfiles(b"d /var/lib/demo 0755\nd /run/demo/x\n", "demo"))
        self.assertFalse(mirror.own_tmpfiles(b"d /var/lib/demo2\n", "demo"))
        self.assertFalse(mirror.own_tmpfiles(b"d /var/lib/demo/../../etc\n", "demo"))
        self.assertFalse(mirror.own_tmpfiles(b"L+ /etc/x - - - - /y\n", "demo"))


    def test_root_reads_what_runs_as_root_and_an_app_keeps_its_own_config(self):
        for n in ["etc/demo/config.json", "etc/demo.conf", "etc/default/demo", "usr/bin/demo",
                  "usr/share/demo/x.py", "opt/demo/run"]:
            self.assertFalse(mirror.root_reads(n, "demo"), n)
        for n in ["etc/sudoers.d/demo", "etc/cron.d/demo", "usr/local/bin/apt-get", "usr/sbin/curl",
                  "usr/share/dbus-1/system-services/x.service", "usr/share/dbus-1/system.d/x.conf",
                  "usr/lib/python3/dist-packages/x.pth", "usr/lib/binfmt.d/x.conf",
                  "lib/modules/6.18/x.ko", "usr/lib/aarch64-linux-gnu/security/pam_x.so",
                  "usr/lib/aarch64-linux-gnu/libnss_x.so.2", "usr/lib/pam.d/login"]:
            self.assertTrue(mirror.root_reads(n, "demo"), n)
        # A name that is a system folder or file is not the app's own.
        for pkg, n in [("sudoers.d", "etc/sudoers.d/zz"), ("cron.d", "etc/cron.d/x"),
                       ("nsswitch", "etc/nsswitch.conf"), ("sudo", "etc/sudo.conf"),
                       ("ld.so", "etc/ld.so.conf"), ("pam", "etc/pam.conf"),
                       ("security", "etc/security/limits.conf"),
                       ("dbus-1", "etc/dbus-1/system.d/x.conf"),
                       ("kernel", "etc/kernel/postinst.d/x"), ("ssl", "etc/ssl/openssl.cnf"),
                       ("network", "etc/network/if-up.d/x"), ("networkmanager", "etc/NetworkManager/x")]:
            self.assertTrue(mirror.root_reads(n, pkg), (pkg, n))


class Root(unittest.TestCase):
    def test_sudo_as_a_command_in_code_asks_for_root(self):
        x = 0o755
        self.assertTrue(mirror.asks_root("opt/app/run.sh", x, b"#!/bin/sh\nsudo reboot\n"))
        self.assertTrue(mirror.asks_root("opt/app/app.py", 0o644,
                                         b"subprocess.run(['sudo', 'ip', 'link'])"))
        self.assertTrue(mirror.asks_root("usr/bin/app", x, b"\x7fELF..\0/usr/bin/pkexec\0"))
        self.assertTrue(mirror.asks_root("usr/bin/helper", 0o4755, None), "setuid")

    def test_a_word_in_data_or_another_name_is_not_root(self):
        x = 0o755
        self.assertFalse(mirror.asks_root("usr/share/doc/app/README", 0o644, b"run with sudo"))
        self.assertFalse(mirror.asks_root("opt/app/notes.txt", 0o644, b"sudo make me"))
        self.assertFalse(mirror.asks_root("usr/bin/app", x, b"\x7fELF pseudo_random sudoku\0"))
        self.assertFalse(mirror.asks_root("opt/app/app.py", 0o644, b"import os\n"))


class Uses(unittest.TestCase):
    def test_a_tool_counts_where_a_shell_runs_it(self):
        sh = ("#!/bin/sh\n"
              "wget -qO - https://x/key | gpg --dearmor > /usr/share/keyrings/k.gpg\n"
              "CN=$(lsb_release -cs)\n"
              "if ! command -v zstd >/dev/null; then exit 1; fi\n"
              "  amixer -c 1 sset 'ADC MUX' AMIC\n"
              "cd build && make -j4\n"
              'V="$(xz --version)"\n'
              "su -c 'sox in.wav out.ogg' pi\n")
        self.assertEqual(mirror.uses_in("sh", sh),
                         {"wget": 2, "gpg": 2, "lsb-release": 3, "zstd": 4, "alsa-utils": 5,
                          "xz-utils": 7, "sox": 8})

    def test_a_tool_named_in_words_or_comments_does_not(self):
        sh = ("#!/bin/sh\n"
              "# git clone https://x and run zstd\n"
              'log "route=$(ip route show default)"\n'
              'echo "lokictl: could not make ${d}; git is fine" >&2\n'
              "python3 x.py\n"
              "mygpg --help\n")
        self.assertEqual(mirror.uses_in("sh", sh), {})

    def test_python_imports_and_the_commands_it_runs(self):
        py = ("import os\nimport numpy as np\n"
              "def cam():\n    from picamera2 import Picamera2\n"
              "subprocess.Popen(['zstd', '-dc'], stdin=raw)\n"
              "os.system(\"amixer -c 1 cset name='x' 1\")\n"
              "import serial\n")
        self.assertEqual(mirror.uses_in("py", py, own={"serial"}),
                         {"python3-numpy": 2, "python3-picamera2": 4, "zstd": 5, "alsa-utils": 6})

    def test_libraries_an_app_carries_are_its_own(self):
        names = ["usr/lib/app/main.py", "usr/lib/app/site/typer/__init__.py",
                 "usr/lib/app/site/typer-0.9.dist-info/METADATA", "usr/lib/app/yaml/__init__.py",
                 "usr/lib/app/sounddevice.py", "usr/lib/app/_cffi.cpython-313-aarch64-linux-gnu.so"]
        self.assertEqual(mirror.vendored_roots(names), {"usr/lib/app/site/"})
        self.assertEqual(mirror.provided_modules(names) & {"typer", "yaml", "sounddevice", "_cffi"},
                         {"typer", "yaml", "sounddevice", "_cffi"})
        self.assertTrue(mirror.VENDORED.search("opt/flint/vendor/x.py"))
        self.assertFalse(mirror.VENDORED.search("opt/vendors/x.py"))

    def test_the_uses_column_reads_back_and_names_only_listed_packages(self):
        row = {"uses": "gpg=DEBIAN/postinst:3;zstd=usr/bin/x:9;Bad Name=x;nmap=a.py:1"}
        self.assertEqual(mirror.uses_of(row), {"gpg": "DEBIAN/postinst:3", "zstd": "usr/bin/x:9"})

    def test_a_files_name_never_adds_an_entry(self):
        x = 0o755
        data = [("./usr/lib/a;nmap=b.py", False, 0o644, b"import numpy\n"),
                ("./usr/bin/c d=e", False, x, b"#!/bin/sh\nzstd -d x\n")]
        u = mirror.uses([], data)
        self.assertEqual(set(mirror.uses_of({"uses": u})), {"python3-numpy", "zstd"}, u)
        self.assertNotIn("nmap=", u)


class FakeDevice(mirror.Device):
    """apt as a table: what each install brings, and what can't install.
    sox installs alone but not with anything else."""

    def __init__(self, deps, sizes, broken=()):
        self.deps, self.sizes, self.broken = deps, sizes, set(broken)

    def simulate(self, row, extras=()):
        if set(extras) & self.broken or "sox" in extras and len(extras) > 1:
            return False, " app : Depends: x but it is not installable\n"
        names = set(self.deps.get(row["package"], ()))
        for e in extras:
            names |= {e} | set(self.deps.get(e, ()))
        return True, "".join(f"Inst {n} (1 Debian:13 [arm64])\n" for n in sorted(names))

    def installed_bytes(self, pkgs):
        return sum(self.sizes.get(p, 1 << 20) for p in pkgs)


class Extras(unittest.TestCase):
    def row(self, uses, depends=""):
        return row("e" * 64, "demo", depends=depends, uses=uses)

    def test_an_extra_is_what_the_app_uses_and_its_dependencies_dont_bring(self):
        dev = FakeDevice({"demo": ["python3", "python3-pil"]}, {})
        kept, rep = dev.extras(self.row("zstd=usr/bin/x:3;python3-pil=a.py:1;gpg=DEBIAN/postinst:9",
                                        depends="python3"))
        self.assertEqual(kept, ["gpg", "zstd"])
        self.assertEqual([r[0] for r in rep], ["gpg", "zstd"])

    def test_one_that_cant_install_or_is_too_big_is_left_out_with_why(self):
        dev = FakeDevice({"python3-opencv": ["libopencv"]}, {"libopencv": 90 << 20,
                                                             "python3-numpy": 20 << 20},
                         broken={"python3-pyaudio"})
        kept, rep = dev.extras(self.row("python3-opencv=a.py:1;python3-pyaudio=b.py:1;"
                                        "python3-numpy=c.py:2;zstd=d:1"))
        self.assertEqual(kept, ["python3-numpy", "zstd"], "numpy is big but BIG_OK")
        why = {p: w for p, w, _ in rep}
        self.assertTrue(why["python3-opencv"].startswith("too big"))
        self.assertTrue(why["python3-pyaudio"].startswith("can't install"))

    def test_extras_that_cant_install_together_are_all_left_out(self):
        dev = FakeDevice({}, {})
        kept, rep = dev.extras(self.row("sox=a:1;zstd=b:1"))
        self.assertEqual(kept, [])
        self.assertEqual({w for _, w, _ in rep}, {"not with the others"})

    def test_a_reviewed_fix_adds_and_drops(self):
        old = mirror.EXTRAS_FIX
        mirror.EXTRAS_FIX = {"demo": {"+bc", "-zstd"}}
        try:
            kept, _ = FakeDevice({}, {}).extras(self.row("zstd=a:1"))
            self.assertEqual(kept, ["bc"])
        finally:
            mirror.EXTRAS_FIX = old

    def test_extras_are_the_lines_last_field_and_only_when_there_are_any(self):
        apps, checked, names, cn = fixture()
        a = next(x for x in apps if x["download"]["package"] == "acidzero")
        plain, _ = mirror.app_line(a, checked["a" * 64], cn["acidzero"])
        line, _ = mirror.app_line(a, checked["a" * 64], cn["acidzero"], ["zstd", "gpg"])
        self.assertEqual(line, plain[:-1] + "\tgpg,zstd\n")
        self.assertEqual(len(plain.rstrip("\n").split("\t")), 15, "app plus 14 fields as before")
        lines, _ = mirror.listing(apps, checked, names, {"gh0st"}, cn, {}, None, {"a" * 64: ["gpg"]})
        self.assertTrue(any(l.startswith("app\tacidzero\t") and l.endswith("\tgpg\n") for l in lines))


@unittest.skipUnless(shutil.which("dpkg-deb"), "needs dpkg-deb")
class Debs(unittest.TestCase):
    def test_facts_see_scripts_paths_units_and_the_desktop_file(self):
        with tempfile.TemporaryDirectory() as d:
            root = os.path.join(d, "root")
            for p in ["DEBIAN", "usr/bin", "usr/share/APPLaunch/applications", "etc/systemd/system",
                      "etc/demo", "usr/lib/tmpfiles.d"]:
                os.makedirs(os.path.join(root, p))
            with open(os.path.join(root, "DEBIAN/control"), "w") as f:
                f.write("Package: demo\nVersion: 1.2-1\nArchitecture: arm64\nMaintainer: x <x@example.invalid>\n"
                        "Depends: libc6\nDescription: demo\n")
            with open(os.path.join(root, "DEBIAN/postinst"), "w") as f:
                f.write("#!/bin/sh\n")
            os.chmod(os.path.join(root, "DEBIAN/postinst"), 0o755)
            for p in ["usr/bin/demo", "usr/share/APPLaunch/applications/demo.desktop",
                      "etc/systemd/system/demo.service", "etc/demo/config.json"]:
                open(os.path.join(root, p), "w").close()
            tmpfiles = os.path.join(root, "usr/lib/tmpfiles.d/demo.conf")
            with open(tmpfiles, "w") as f:
                f.write("# its state\nd /var/lib/demo 0755 root root -\n")
            deb = os.path.join(d, "demo.deb")

            def build():
                subprocess.run(["dpkg-deb", "--root-owner-group", "-b", root, deb],
                               check=True, capture_output=True)
                return mirror.facts(deb, "0" * 64)

            f = build()
            self.assertEqual((f["package"], f["version"], f["arch"]), ("demo", "1.2-1", "arm64"))
            # A unit is a service, its own /etc folder and tmpfiles are fine.
            self.assertEqual((f["scripts"], f["paths"], f["service"]), ("1", "0", "1"))
            self.assertEqual((f["depends"], f["desktop"]), ("libc6", "demo"))
            with open(tmpfiles, "a") as t:
                t.write("z /etc/shadow 0644 root root -\n")
            self.assertEqual(build()["paths"], "1", "tmpfiles past its own folders")
            with open(tmpfiles, "w") as t:
                t.write("d /var/lib/demo 0755\n")
            os.makedirs(os.path.join(root, "etc/sudoers.d"))
            open(os.path.join(root, "etc/sudoers.d/demo"), "w").close()
            self.assertEqual(build()["paths"], "1", "a file root reads")
            self.assertEqual(f["root"], "0")
            with open(os.path.join(root, "usr/bin/demo"), "w") as b:
                b.write("#!/bin/sh\nexec sudo /opt/demo/run\n")
            os.chmod(os.path.join(root, "usr/bin/demo"), 0o755)
            self.assertEqual(build()["root"], "1", "a script that calls sudo")
            self.assertEqual(f["keys"], "0")
            with open(os.path.join(root, "usr/bin/demo"), "w") as b:
                b.write("#!/bin/sh\nd=$HOME/.config/systemd/user/APPLaunch.service.d\n"
                        "echo Environment=LD_PRELOAD=/usr/lib/demo/bridge.so > $d/90-demo.conf\n")
            self.assertEqual(build()["keys"], "1", "an input method's setup")
            self.assertEqual(f["uses"], "")
            with open(os.path.join(root, "DEBIAN/postinst"), "w") as p:
                p.write("#!/bin/sh\nwget -qO- https://x/k | gpg --dearmor > /usr/share/keyrings/k.gpg\n")
            os.makedirs(os.path.join(root, "usr/lib/demo/site/yaml-6.dist-info"))
            open(os.path.join(root, "usr/lib/demo/site/yaml-6.dist-info/METADATA"), "w").close()
            with open(os.path.join(root, "usr/lib/demo/site/lib.py"), "w") as p:
                p.write("import rich\n")
            with open(os.path.join(root, "usr/lib/demo/app.py"), "w") as p:
                p.write("import numpy\nimport lib\n")
            self.assertEqual(build()["uses"],
                             "gpg=DEBIAN/postinst:2;python3-numpy=usr/lib/demo/app.py:1;"
                             "wget=DEBIAN/postinst:2",
                             "a vendored library's imports don't count")


if __name__ == "__main__":
    unittest.main()
