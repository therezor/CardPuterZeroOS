# Hotspot

Settings > Hotspot turns the device into a Wi-Fi access point. Phones and
laptops join it to reach the device (SSH, files), and the internet when a
cable is up. It takes wlan0, so the device leaves its Wi-Fi network while
the hotspot runs and rejoins it when the hotspot stops.

## How it works

NetworkManager runs the access point on wlan0 in `shared` mode. Shared mode
brings dnsmasq for DHCP and DNS on 10.42.0.1, and an nftables table that
masquerades the clients out whatever link is up. iw counts the clients
(`iw dev wlan0 station dump`) and sets Wi-Fi power saving. We run no daemon.

| Piece | Where |
|---|---|
| The profile: render, parse, checks, the QR text | `crates/gh0st-core/src/hotspot.rs` |
| Root verbs: `hotspot on\|off`, `name`, `password`, `security` | `crates/gh0st/src/helper.rs` |
| The pages and the jobs | `crates/gh0st-settings/src/model/pages.rs`, `src/app.rs` |
| "Is wlan0 an access point": `SIOCGIWMODE` | `crates/gh0st-core/src/status.rs` |
| The QR code | `crates/gh0st-ui/src/qr.rs`, `widgets::qr_card` |

The helper is the only writer of the keyfile,
`/etc/NetworkManager/system-connections/gh0st-hotspot.nmconnection`. It makes
one with a neutral name and a random password the first time anything needs
it. It runs nmcli as root, so polkit needs no `wifi.share` rule. A change
while the hotspot runs restarts it, so clients join again.

The verbs need no password. They carry the trust the device user already has
over NetworkManager through our polkit rule (`packaging/polkit/50-gh0st.rules`).

## Settings

| Row | Does |
|---|---|
| Hotspot | On and off. "Starting…" while the helper works |
| Name | 1 to 32 bytes; default `zero-` and four letters |
| Password | 8 to 63 characters, no spaces; default 12 random lowercase letters |
| Security | WPA3 (default), or WPA2/WPA3 for older devices |
| Show QR code | A `WIFI:` code a phone camera joins with, the name and password beside it, and this device's address |
| Devices | How many have joined, while on. R reloads it |

While the hotspot runs, the Wi-Fi page shows "Hotspot is on" in place of the
networks, the top bar shows a radio tower, and going dark leaves the radio on.

Wi-Fi > Power saving sets `wifi.powersave` for every connection, and the
helper applies it to the link up now with `iw dev wlan0 set power_save`.

## Security

- WPA3 (SAE) with protected management frames required, by default. Mixed
  mode is a choice. There is no open network, no TKIP, no WPS.
- The password goes to the helper on stdin, never in an argument. The keyfile
  is mode 0600, root's. Settings wipes the copy it holds.
- `autoconnect=false`: off after every boot. The image ships no profile.
- The access point's address is a stable local one, not the chip's.
- The profile asks for client isolation (`ap-isolation=1`), and IPv6 is off
  on the hotspot.
- Clients reach DHCP and DNS on 10.42.0.1, sshd only while SSH is on, and the
  internet only over a cable. NetworkManager's `nm-shared-wlan0` table is
  there to reject new connections from upstream into 10.42.0.0/24.
- Isolation and the upstream reject are configured, not yet checked on the
  device: see the table below.

## Why not alongside Wi-Fi

The chip lists one client and one access point at once, on one channel. A
second interface (`iw dev wlan0 interface add uap0 type __ap`) does start
next to the Wi-Fi link. On the device it was not usable:

- WPA3 failed both ways. Clients could not join over SAE, and wlan0 could not
  rejoin a WPA3 network while the access point ran (`brcmf_cyw_mgmt_tx` errors).
- A scan on wlan0 disconnected the access point's clients.
- Restarting the access point dropped wlan0 (`brcmf_cfg80211_stop_ap: -52`).

On its own on wlan0, WPA3 and mixed mode both work. Results are in
`hardware-facts.md`.

## Tests

Automated, in `cargo test --workspace`:

- The profile round-trips, including an SSID with `;` and Cyrillic and a
  password with `\`. Both security modes write their fixed lines.
- Names and passwords at their limits, the password generator, the client
  count, the QR escaping.
- The helper's verbs, good and bad, and that they need no password.
- The top bar: an access point is the hotspot, never Wi-Fi bars, even over a
  cable.
- The pages: rows for each state, Wi-Fi pointing to the hotspot while it
  runs, the profile kept out of the saved networks.
- `tests/hotspot_sim.rs`: the real Settings binary against fake pkexec, nmcli
  and iw. It turns the hotspot on, renames it, refuses a 7-character
  password, sets a new one and changes the security, and checks the password
  never reaches an argument.
- The busy card: shown only past its grace, keys held, Esc hides it.

Mockups: `cargo run -p gh0st-settings --example mockups` writes the hotspot
pages to `mockups/settings/*hotspot*`.

On the device (checked on the stock card over a cable, unless marked):

| Check | Result |
|---|---|
| On through pkexec, as the device user, no password | 2.4 s, keyfile 0600 root, SAE, PMF required, channel 11 |
| iPhone and POCO X7 Pro join over WPA3 | yes, both by password (on the spike profile) |
| Internet through the cable | not confirmed: both joined, no page load reported |
| Off rejoins the saved network | yes |
| Join by QR, iPhone and Android | not yet |
| Clients can't reach each other | not yet |
| From upstream, 10.42.0.0/24 can't be reached | not yet |
| `ss` and `nft` back to baseline after off | not yet |
| Reboot leaves it off | not yet |
| Our image: iw, dnsmasq-base and nftables come with the gh0st deb | `os/build.sh check release` passes: 451 packages, 845 of 850 MB, `iw` resolves, no Wi-Fi profile in the image |
| The hotspot running on our image | not yet |
| Idle cost with the hotspot off equals the baseline (`measure.sh`) | not yet |
