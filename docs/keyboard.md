# Keyboard

Keyboard makes the CardputerZero a keyboard, mouse and media remote for a
computer or a phone, over Bluetooth LE or a USB cable. It is an SDK app,
`crates/gh0st-keyboard`, with its own binary, `gh0st-keyboard`.

## Screens

- **Hosts**, the first screen. The USB cable first, then every paired
  computer or phone, the one used last on top. Enter types on the selected
  one. Chips: Pair, Unlink (while the host is connected), and Forget, held.
  The menu adds host settings and turns the USB keyboard on or off.
- **Pair.** The keyboard is discoverable, with the device's name, only while
  this screen is open, for 3 minutes at most. The host's pairing code shows
  large: Enter when it matches, Esc when not. A host that asks for a code
  typed here gets a field.
- **A host's page**: its name, its system and "Stay connected behind apps".
  It opens after pairing with "Start typing" first.
- **The session**, in five modes. Fn+Space lists them; a digit or Enter
  picks one. The app remembers the last.

| Mode | What it does |
|---|---|
| Type | Every key goes to the host. A line shows what was sent |
| Mouse | F Z X C or the arrows move, faster while held, and faster still with Shift. Space or Enter clicks or drags, Bksp is the right button, P the middle, R and V scroll |
| Media | Enter plays or pauses, z and c skip, f and x change the volume, m mutes, s is a phone camera's shutter (volume up) |
| Slides | Space, Enter or c for the next slide, Bksp or z for the previous, b black. Start sends F5, or Keynote's Option Cmd P on a Mac. A timer counts while it runs |
| Macros | The host system's shortcuts (copy, lock screen, app switcher, screenshot), then saved texts. Enter sends one. A text is typed at about 40 characters a second; Esc stops it |

## Keys in Type

Esc goes back to the hosts, as everywhere in the OS. Fn+Esc sends Esc.

| Keys | The host gets |
|---|---|
| Letters, digits, Enter, Tab, Bksp, Space | the same key, with Shift, Ctrl and Alt as held |
| Sym and a key | its symbol, with Shift set as the symbol needs |
| Fn and F Z X C | the arrows |
| Fn and L, M, N | Page Up, Page Down, End |
| Fn and Tab | Home (Fn+K stays the OS's app switcher) |
| Fn and Bksp | Delete |
| Fn and Q, W, E | play or pause, previous, next |
| Fn and 1 to 0, O, P | F1 to F12 |
| Fn then Alt | Win, Super, Search, or Option on a Mac, with the next key. Twice: that key alone |
| Alt, on a Mac or an iPhone | Cmd |

The keyboard is a US keyboard, and says so to every host: one layout. A
host set to another layout types its own characters for these keys, as any
US keyboard would there.

The OS keeps Fn+K, Fn+H and the volume, brightness and screenshot keys.
Help, the menu, dialogs, Fn+K and the screen going dark let go of every
key the host holds.

## Bluetooth

HID over GATT through BlueZ, on the system bus as the user. No root.

- The GATT tree: HID service 0x1812 with a report map of three reports
  (keyboard, consumer control, mouse, each its own ID), boot keyboard in
  and out, and Battery 0x180f. Reads need an encrypted link, so a host must
  pair before it sees the keyboard.
- Reports go to BlueZ as a PropertiesChanged of the report's Value, sent
  to BlueZ's name alone.
- The advert runs only while the app is in front and waiting for a host:
  discoverable with the name while pairing, nameless for a bonded host to
  come back, for 2 minutes. None runs while a host is connected.
- The agent shows pairing codes and refuses pairing outside the Pair screen.
- Classic Bluetooth (BR/EDR) is off while the Pair screen is open
  (`gh0st-helper bt-classic off`, then `on`, through the kernel's management
  socket). Android, seeing a classic side on a device it pairs with over LE,
  looks for a classic keyboard there and never starts the LE one. Pairing
  with classic off makes it an LE keyboard for good; classic comes back
  after. The switch restarts the radio, which drops classic connections.
- A phone pairs from a private address and then gives its own; BlueZ keeps
  the device and changes its address. The host follows the change, so a
  host is always saved under its own address.
- A computer comes back by itself when the keyboard advertises. A phone
  waits for a tap on the keyboard in its Bluetooth settings, and the
  waiting screen says so.
- An iPhone uses the mouse only with AssistiveTouch on.
- One host types at a time. Picking another lets go of the first.
- Devices paired in Settings, such as headphones, are not hosts: the app
  never disconnects them.
- Bluetooth turns on when the app needs it and off on exit if the app turned
  it on.
- Hosts see the battery level, sent on connect and when the app returns.

## USB

With the side switch on Slave, the right-hand USB-C port is a USB device.
That cuts the hub: the USB-A port, Ethernet and the other USB-C.

`gh0st-helper usb-hid hold` (root, through pkexec, no password) builds a
configfs gadget with three HID functions: a boot keyboard with its lights,
a boot mouse and consumer control. It binds the board's controller, gives
each `/dev/hidgN` to the user who asked (mode 0600), prints `node NAME PATH`
for each and `state WORD` as the controller's state changes, and takes the
gadget down when its stdin closes. The app keeps that pipe, so however the
app ends, the host sees the keyboard unplugged and lets go of every key.

A node takes one report until the host collects it. The app keeps a queue
per node and tries again every 4 ms, only while reports wait.

| Controller state | The app shows |
|---|---|
| configured | Connected |
| suspended | The host is asleep |
| not attached, with the hub on the bus | Set the side switch to Slave |
| anything else | Plug into a computer or phone |

The gadget is `1d6b:0104`, "Gh0st Keyboard" by "Gh0st", serial `gh0st`:
nothing in it names the device or its owner.

## Leaving the front

On Fn+K or the dark the app lets go of every key. If it may run behind
(the default, Settings > Apps), a computer stays connected and a phone is
let go, so its own keyboard comes back; the host's page changes that. If it
may not, it unregisters from BlueZ and stops the USB keyboard first.

## Tests

- `tests/engine.rs`: keys to reports, Sym characters, the latch, the OS
  keys, letting go.
- `tests/ble.rs`: the link against a fake BlueZ (`tests/common/fakebluez.rs`).
- `tests/usb.rs`: the queues against a node that refuses, and the helper's
  protocol against a stand-in script.
- `tests/pages.rs`: the screens on fake links: what reaches the host, the
  Esc rules, letting go, no wakes while idle.
- `examples/mockups.rs`: every screen into `mockups/keyboard`, linted in
  every palette.
- `examples/probe.rs`: the Bluetooth link on the device without the app. It
  pairs and types a line.
