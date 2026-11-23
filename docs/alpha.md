# Gh0st OS alpha: notes for testers

Gh0st OS for the M5Stack CardputerZero. This is an alpha build. It is ready to
use every day, but it can still break, and the SDK and the design can still
change. Flashing it erases the card.

## What you need

- A CardputerZero.
- A genuine microSD card, 8 GB or larger, from a seller you trust. Fake cards
  claim more space than they have and corrupt your files once they fill past
  their real size.
- [Raspberry Pi Imager](https://www.raspberrypi.com/software/) 2.0.2 or later,
  on Windows, macOS or Linux. Older versions have a storage erasure bug on
  macOS and Linux.

## Flash it

1. From the [release page](https://github.com/therezor/CardPuterZeroOS/releases),
   download `gh0st-release-DATE.img.xz` and `gh0st-release-DATE.sha256`.
2. Optional: check the download. In the folder that holds both, run
   `shasum -a 256 -c --ignore-missing gh0st-release-DATE.sha256` (macOS) or
   `sha256sum -c --ignore-missing gh0st-release-DATE.sha256` (Linux).
3. In Imager, for the device pick Raspberry Pi Zero 2 W (the same chip), or
   no filter.
4. For the OS, scroll to **Use custom** and pick the `.img.xz`. Do not unpack
   it.
5. For the storage, pick the card. Check the size: Imager erases what you pick.
6. If Imager offers OS customisation, answer **No**. Its script is made for
   Raspberry Pi OS, and setup on the device does that job.
7. Write, and let Imager verify.

On a Mac with this repository, `scripts/flash.sh` does the same from the
terminal. It refuses internal disks and asks before it writes:

```sh
diskutil list                                          # find the card: /dev/diskN
scripts/flash.sh gh0st-release-DATE.img.xz /dev/diskN
```

## First boot

Put the card in and power on. The M5Stack logo shows, then setup, within
about 15 seconds. On the first boot the device grows its data partition to
fill the card.

Setup, once:

1. An alpha notice. Enter goes on.
2. Welcome. Enter starts a short training of the keys. 6 skips it.
3. The training, 9 short lessons. Skip on a lesson goes to the next one, and
   Skip training in the menu (key 8) skips the rest.
4. The settings: country, time zone, privacy, Wi-Fi, the date and time (only
   when the network has not set the clock), your account, the device name
   and look.
5. The home screen.

## Keys

| Key | What it does |
|---|---|
| Enter | Go on, open |
| Esc | Back |
| 4 to 8 | The actions named above them. 8 opens the menu |
| Fn+H | Help: every key on this screen |
| Fn+K | Switch between open apps |
| Hold Esc | Close the app in front |
| Hold Tab | Lock |
| Tab, then Enter | Unlock |

## Private by default

The Wi-Fi radio stays off until setup's Wi-Fi step. Bluetooth, SSH and logs are
off. Turn them on in Settings: SSH in System, logs in Privacy.

## Updates

The device checks once a day and asks before it installs. Updates opens from
the notification or from home. An update keeps your files and settings; only a
new flash erases the card.

## Report a bug

Open an issue on GitHub and include:

- The **Build** from Settings > About.
- What you did, what you expected, and what happened. A photo of the screen
  helps.
- For a crash or a hang: turn logs on in Settings > Privacy, do it again, then
  turn SSH on and save the output of `sudo journalctl -b`. It holds your
  account name, network names and addresses: remove them, or send the log
  privately.

## Known issues

- Every card made from this image has the same disk ID. Do not plug two of
  them into one Linux computer at the same time.
- Wi-Fi is 2.4 GHz only: that is the chip. Its firmware has no monitor
  mode. A USB adapter can do it: the image carries firmware for the common
  monitor-capable chips (Atheros AR9271, Ralink RT5370, MediaTek MT7601U and
  MT7612U, Realtek RTL8812AU and RTL8811AU, and more; `gh0st-wifi`).
- The hotspot takes the Wi-Fi: the device leaves its network while it runs.
- The training is part of setup for now. It becomes its own app later.
- The stock Camera app is still on the image next to ours, so home shows two.
