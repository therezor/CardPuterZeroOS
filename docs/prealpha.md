# Gh0st OS pre-alpha: notes for testers

Gh0st OS for the M5Stack CardputerZero. This is a pre-alpha test build. It can
break, and the SDK and the design can still change. Flashing it erases the
card.

## What you need

- A CardputerZero.
- A genuine microSD card, 8 GB or larger, from a seller you trust. Fake cards
  claim more space than they have and corrupt your files once they fill past
  their real size.
- Raspberry Pi Imager 2.0.2 or later. It fixes a storage erasure bug on macOS
  and Linux.

## Flash it

1. Download `gh0st-os-prealpha-DATE-BUILD.img.xz` and its `.img.xz.sha256`.
   To check the download, run `shasum -a 256 -c` on the `.img.xz.sha256` file
   (macOS) or `sha256sum -c` (Linux), in the folder that holds both.
2. In Imager, for the device pick Raspberry Pi Zero 2 W (the same chip), or
   no filter.
3. For the OS, scroll to **Use custom** and pick the `.img.xz`. Do not unpack
   it.
4. For the storage, pick the card. Check the size: Imager erases what you pick.
5. If Imager offers OS customisation, answer **No**. Its script is made for
   Raspberry Pi OS, and setup on the device does that job.
6. Write, and let Imager verify.

## First boot

Put the card in and power on. The M5Stack logo shows, then setup, within
about 15 seconds. On the first boot the device grows its data partition to
fill the card.

Setup, once:

1. A pre-alpha notice. Enter goes on.
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
| 5 to 8 | The actions named above them. 8 opens the menu |
| Fn+H | Help: every key on this screen |
| Fn+K | Switch between open apps |
| Hold Esc | Close the app in front |
| Hold Tab | Lock |

## Private by default

The Wi-Fi radio stays off until setup's Wi-Fi step. Bluetooth, SSH and logs are
off. Turn them on in Settings: SSH in System, logs in Privacy.

## Report a bug

Include:

- The **Build** from Settings > About.
- What you did, what you expected, and what happened. A photo of the screen
  helps.
- For a crash or a hang: turn logs on in Settings > Privacy, do it again, then
  turn SSH on and save the output of `sudo journalctl -b`. It holds your
  account name, network names and addresses: remove them, or send the log
  privately.

## Known issues

- No updates yet. To get a newer build, flash it again, which erases the card.
- Every card made from this image has the same disk ID. Do not plug two of
  them into one Linux computer at the same time.
- Wi-Fi is 2.4 GHz only: that is the chip. No hotspot or monitor mode yet.
- Setup has no wired network yet: "Use the cable" is not built.
- The training is part of setup for now. It becomes its own app later.
