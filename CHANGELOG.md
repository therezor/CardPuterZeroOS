# Changelog

## v0.5.0 beta

- 🎧 **Bluetooth headphones and speakers.** Pair them in Settings and play
  through them. They connect again when Bluetooth comes back on.
- 🏠 **Row, a new home layout.** One large app in the middle with its
  neighbours either side, like the stock launcher. Apps with their own
  picture fill their card.
- 🔌 **The side switch shuts down properly.** The screen says it is
  shutting down, and nothing is lost. Hold Esc on home to restart.
- 🔒 Hold Tab to lock over an app too. The lock screen slides away as you
  unlock.
- 📁 Files can open protected folders after you type your password.
- 🔐 **Apps that run as root ask for the password through the shell.** A
  Store app that calls `sudo` gets the shell's own prompt, and the keyboard
  stays the shell's while it shows. An app that quits at once leaves a
  notice that says why.
- 🛍️ Every app in the Store installs.
- 🧪 Try what comes next: Settings > System > Updates > Channel > Edge.
- 🐛 Fixes: Bluetooth stays off after a restart, notices over apps no
  longer flicker, and network apps from the Store find the Ethernet port.

## v0.4.0 beta

- 🛡️ **Apps kept apart.** Every app runs in a sandbox of its own. It can't
  read your SSH keys or passwords, change what your terminal runs, or see
  other apps.
- 🎚️ **Network, camera and microphone per app.** Turn any of them off for
  an app in Settings > Apps.
- 🛍️ The Store marks apps that need full access to your device, and asks
  before it installs one.
- 📷 **Film looks** for photos in Camera, and a shutter that answers while
  the last photo saves.
- 🔔 The switcher shows a bell when notifications wait.
- 🌍 Setup and Settings find your country as you type its name.
- 🐛 Fixes: Camera and video use less memory, the first sound after the
  screen wakes plays whole, an app that pauses late comes back, and
  Settings shows whether logs are really kept.

## v0.3.0 alpha

- 📁 **Files.** Browse the card, open zip files like folders, and connect to
  your servers. Any file opens in the right app.
- 📝 **Text.** A simple editor for notes and code. It opens files of any size
  and brings back unsaved changes after a crash.
- 🖼️ **Media.** Photos, videos and music in one app. Music keeps playing while
  you use other apps.
- ⌨️ **Keyboard.** Use the CardputerZero as a keyboard, mouse and remote for
  your computer or phone.
- 🔑 **Keyring.** Keeps your passwords safe. Apps ask before they read one.
- 📋 **Copy and paste** between apps. Locking clears what you copied.
- 🔊 **Sounds.** Soft clicks as you move and choose, and a shutter sound in
  Camera. Turn them off in Settings.
- 🌐 **Network details** in Settings, updated as soon as a connection changes.
- 🔁 Files and Media replace M5's Files and Music apps.
- 🐛 Many fixes, including a way past the lock screen.

Known issue: after some hours, a photo can fail to save. Restart the device.

## v0.2.0 alpha

- 🎨 A new look with sharper text and actions shown above their keys.
- 🖼️ Wallpapers, including your own photos.
- 🔒 A lock screen with the time and your notifications count.
- 🛍️ A better Store with categories, search and install progress.
- 📷 Our own Camera, with photos and 1080p video.
- 🧰 Command-line tools for the pins and the Grove port.

## v0.1.0

The first release.
