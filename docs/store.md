# The Store

The Store installs CardputerZero apps: the ones M5Stack lists in its own
AppStore, from the same packages. It replaces M5's AppStore binary
(`gh0st-m5-appstore`), which the image carries until this ships.

## What was wrong with M5's

Read from its source (`CardputerZero/Store` at 1ceeaed7, the commit
launcher-v0.6.111 pins) and measured on the device (`hardware-facts.md`):

- It takes about 100 s from open to the list, and holds 57 MB PSS there.
- On screen and idle, one thread spins a whole CPU core.

- **On every open** it wipes its cache and downloads the 754 KB catalog
  again, uncompressed and with no ETag. Then it fetches all 179 icons, 6.6 MB
  of PNGs up to 512 px, and decodes each in full.
- It runs `dpkg-query` twice per app: 358 processes, about 30 s on a device.
- It loads two Noto CJK faces, 46 MB of font files. The serif one is unused.
- It keeps every deb it downloads, forever.
- The default catalog URL redirects to plain HTTP. The only check on a deb
  is an MD5 from that same unsigned file. The catalog's SHA-256 is ignored.
- It asks the password for every action, and shows one app per screen.
- "Update available" is a string compare, so an older version counts too.

## How ours works

```text
M5's registry.json ──CI, daily──> catalog + catalog.sig, about, icons
                                     (our Pages, next to the apt repo)
device: gh0st store refresh ──> checks the signature with sqv ──> /var/lib/gh0st/store
        gh0st store install ──> deb from M5's release ──> SHA-256 ──> apt-get install ./x.deb
Store app: reads those files, watches them with inotify, asks gh0st-helper
```

- **The catalog is ours, signed.** CI reads M5's registry and writes a small
  line format, signed with the apt repo's key. The device checks it with
  `sqv`, apt's own verifier, against `gh0st-archive-keyring.pgp`. A catalog
  older than the one on the device, or past its `expires` date, is refused.
- **Debs come from M5's GitHub release**, or their mirror in China. The
  catalog pins each by SHA-256 and size, so the mirror doesn't matter.
- **The device downloads little.** The catalog and the icon pack (48 px)
  and descriptions, which come only when their hash changes. Screenshots come one at a time, when the user opens
  them, checked by SHA-256 and kept in a 4 MB cache.
- **No network at open.** The Store shows the list it has. It refreshes
  when the user asks, and with the daily update check when that is on.
- **apt installs.** `apt-get install ./app.deb` resolves dependencies from
  our sources. A deb is deleted once installed.

### What the signature does not do

It proves the catalog is what M5 published, unchanged on the way. It does
not vet an app. M5 merges version updates without review, and a package's
maintainer scripts run as root. CI flags what it can see, and the detail
page says it:

| Flag | Seen how | Shown as |
|---|---|---|
| `scripts` | the deb has `preinst`, `postinst`, `prerm` or `postrm` | Setup runs as root |
| `paths` | a file where root reads and acts: `/etc` outside `/etc/<pkg>`, udev rules, sysctl, modprobe, polkit, NetworkManager, `/boot`, tmpfiles that reach past the app's own folders, or a setuid file | System files |
| `service` | M5's `background_service`, or a systemd unit in the deb | Service |
| `network`, `camera`, `microphone`, `hardware`, `display`, `imu` | M5's permissions | Uses network, camera |

dpkg never lets a package overwrite another's files, so a file elsewhere
is only data. Installing an app with any of the first three asks first, and
names them.
An update doesn't ask.
CI drops an app whose dependencies our sources don't have (14 of 179 on
2026-11-23: Pi camera tools, `raspi-utils-core`, M5's launcher).

## The mirror in CI

`os/store/mirror.py` runs in the build container (`os/build.sh store`),
after the apt publish in `.github/workflows/apt.yml`. Pages deploys one
artifact, so every run of that workflow carries both. It treats M5's
registry as untrusted:

- **Sources.** The registry from `raw.githubusercontent.com`, over HTTPS
  (M5's own URL redirects to plain HTTP). Their China copy gives `cn-url`,
  taken only when its SHA-256 matches. Every URL must be HTTPS on a known
  host: M5's GitHub release for debs, their OSS bucket in China,
  `raw.githubusercontent.com/CardputerZero/` for art. Redirects to HTTP are
  refused.
- **Debs.** Each new deb downloads once, is checked against the registry's
  size and SHA-256, and only then does `dpkg-deb` list it. Nothing is
  extracted. The facts carry over in `checked.tsv`, by SHA-256. Package,
  version and architecture come from the deb's control file.
- **Left out**, with the reason in `dropped.tsv`: a package named like one
  of Debian's or ours, one whose `Provides`, `Replaces`, `Conflicts` or
  `Breaks` names one, and one with a `Depends` group nothing in trixie or
  our repo offers. Version constraints are not checked.
- **Text.** Control characters go, fields are capped, markdown becomes
  plain paragraphs. A field in Chinese or Japanese is taken from M5's `en`
  locale when there is one.
- **Art.** Icons are decoded under a pixel cap and saved again at 48 px.
  Screenshots are pinned only if they are 320x170 PNGs under 2 MB. Both are
  fetched again only when their ETag changes (`sources.tsv`).
- **Unchanged means unchanged.** When the apps are the same and the
  catalog has more than 20 days left, the old catalog and signature are
  served again, byte for byte. `about` and `icons` are served as
  `about-<sha256>` and `icons-<sha256>`, and the previous pair stays a day,
  so a cached catalog never names a missing file.
- **A bad run changes nothing.** On any error, or when the list shrinks
  below half, the live store site carries over as it was, all of it. The
  step fails only when it can't fetch that either.
- **Signed** with the apt key, as `publish.sh` signs, then checked with
  `sqv` against `os/repo/gh0st-archive-keyring.pgp` before it is kept.

`os/store/test_mirror.py` turns a fixture registry into `testdata/`, and
`gh0st_core::store`'s tests parse the same files. A dry run with a
throwaway key: `STORE_KEYRING=/out/key.pgp REPO_KEY=... STORE_ONLY=acidzero
os/build.sh store`.

## Files

All in `/var/lib/gh0st/store`, written only by the engine, as root:

| File | What |
|---|---|
| `catalog` | the apps; checked against `catalog.sig` before it lands |
| `about` | each app's description and screenshots |
| `icons` | the icon pack |
| `state` | the last run: `refreshed`, `ended`, `op`, `outcome`, `packages`, `failed`, `detail` |

`/run/gh0st-store` holds `progress` (the update engine's format) and `job`,
what the helper asked for. Debs download to `/var/cache/gh0st/store`.

### The catalog

UTF-8, one record a line. Unknown keys are skipped, so an older device
reads a newer catalog. A bad `app` line fails the whole file.

```text
gh0st-store 1
date 1763870000
expires 1766462000
about <sha256> <bytes>
icons <sha256> <bytes>
app	<package>	<version>	<size>	<sha256>	<category>	<flags>	<title>	<summary>	<author>	<licence>	<url>	<cn-url>	<share>	<desktop>
```

`app` fields are tab-separated. `flags` is a comma list from the table
above, or `-`. `desktop` is the stem of the `.desktop` file the deb
installs, so Enter can open the app. `gh0st_core::store` reads and writes
it.

`about` has one app a line: the package, its description with `\n` for a
paragraph, then `<sha256> <url>` for each screenshot, all tab-separated.

`icons` is `gh0st-icons 1`, then `<package> <offset> <length>` lines, a
blank line, and the PNGs back to back.

## The engine and the helper

`gh0st store job` runs as root in `gh0st-store.service`, as the update
engine does. It takes the update engine's lock without waiting: when that
is held, the run ends as `locked`. The app never runs it itself.

| Helper verb | Password | Does |
|---|---|---|
| `store-refresh` | no | fetches and checks the catalog |
| `store-install [--mirror auto\|github\|china] PKG...` | yes | installs or updates |
| `store-remove PKG` | yes | removes, refusing if apt would remove anything else |
| `store-upgrade [--mirror ...]` | yes | updates every app with a newer version |

The helper checks each name against Debian's grammar and the verified
catalog, refuses while an engine runs, writes `job` and starts the unit.
The engine reads `job` once and deletes it. The password has the helper's
5-minute grace, so a run of installs asks once. Debian version order
(`gh0st_core::debver`) decides what is an update.

- **Refresh.** curl fetches `catalog` and `catalog.sig` (HTTPS only,
  compressed), `sqv` checks them, then `about-<sha256>` and `icons-<sha256>`
  come down only when the files on disk don't match. Everything lands in a
  temporary folder first and the catalog moves in last, so a failure leaves
  the old set whole. A catalog no newer than the one on disk changes
  nothing. A signature dated past the clock reads as `clock`, not as a bad
  signature. The site is the apt source's URI with `/store` for `/apt`.
- **Install.** A catalog a day old is refreshed first, and an expired one
  is never used. Each app is its own apt transaction: the deb downloads to
  `/var/cache/gh0st/store/<sha256>.part` (root only, resumed after a
  failure), must match the catalog's SHA-256 and, by `dpkg-deb -f`, its
  package and version. Then `apt-get -s install --no-remove ./<sha256>.deb`
  checks dependencies and space, and the real run follows. Old lists are
  fetched again when a dependency is missing. The deb is deleted after.
- **Remove.** Only an app the catalog lists, and only when apt's simulation
  removes nothing else.
- `state` gains `ended`, so two runs that end alike still differ.

Tests: `crates/gh0st/tests/store_sim.rs` runs the engine with a fake curl,
sqv and apt-get. Real installs were checked as root in the build container:
acidzero with its dependencies on empty lists in 23 s, zero-weather from the
China mirror, an unlisted app and a system package refused, a removal.

## The app

`crates/gh0st-store`, binary `gh0st-store`. `examples/mockups.rs` renders
every screen into `mockups/store`.

**List.** Several apps on screen: icon, name, summary, and a tag for
Installed, Update, a percent or Queued. Left and Right step through All,
Updates, Installed and the categories. With updates waiting the Store opens
on Updates. Typing searches every app by name, share code, summary and
author.

| Key | List | App page | Screenshot |
|---|---|---|---|
| Enter | open the app's page | install, update or open | try again |
| Left, Right | the filter | | the previous or next one |
| Up, Down | move | scroll the description | as Left and Right |
| 5 | Find (on Updates) | Shots | |
| 6 | Find | the one chip there | |
| 7 | Update all (on Updates) | Delete, held | |
| s, u, r | find, update all, refresh | | |
| Esc | leave the search | back to the list | back to the page |

On an app's page the top bar names the app. The menu (8) has Find, Update
all, Refresh and Download from (Auto, GitHub, China; `gh0st.store.mirror`).
Help opens with an About section that says Gh0st doesn't review the apps. Auto picks China's mirror in a Chinese time
zone, as M5's Store does, with no network call.

An install runs in a system unit, so it carries on when the user leaves the
page or closes the app. Installs asked for meanwhile queue in the app. On
its first start the Store deletes M5's cache, `~/.cache/cardputerzero-appstore`,
which held every deb it downloaded, and says how much came back.

**Budgets.** A first frame from the cache in under 150 ms. No wakeups at
idle. Icons decode only for rows near the selection, 20 px, at most 24 kept.
One screenshot in memory. No CJK font. Titles are in English for now; M5's
`zh-CN` and `ja` text is a later step. PSS is measured against M5's Store
with `scripts/measure.sh` and recorded in `hardware-facts.md`.
