# Roadmap

1. **Outside keyboards.** Pair and connect Bluetooth and USB keyboards to the
   device, and pick a layout. (The Keyboard app is the other way round: the
   device as a keyboard for a computer or phone, `docs/keyboard.md`.)
2. **File manager.** Browse, open, copy, move and delete files on the card.
3. **App isolation.** Every app runs as UID 1000, the same user as the shell,
   with the network, the camera, the mic and the home directory open to it.
   In steps:

   | Step | How |
   |---|---|
   | Network off per app | Each app in its own cgroup, and an nftables `socket cgroupv2` drop rule for it |
   | Hide private files | A mount namespace with tmpfs over `~/.ssh` and other apps' data |
   | Camera per app | A BPF device program on the app's cgroup |
   | Mic per app | PipeWire client permissions |
   | Enforce the rules | Root-owned app cgroups, Yama, and a UID per app or Landlock |
