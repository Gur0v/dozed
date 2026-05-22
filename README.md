# dozed

A lightweight idle management daemon for Wayland compositors, and a drop-in
replacement for [swayidle](https://github.com/swaywm/swayidle). Compatible with
any compositor that implements the
[ext-idle-notify](https://gitlab.freedesktop.org/wayland/wayland-protocols/-/tree/main/staging/ext-idle-notify)
protocol. See the man page, [dozed(1)](./dozed.1.scd), for full configuration
reference.

dozed is a fork of swayidle. The command model and general behavior follow
swayidle's design, but most of the implementation has been rewritten in Rust.
Existing swayidle configs and command-line invocations work with dozed without
modification.

## Why dozed?

The main reason for the fork is fullscreen-aware idle behavior. With swayidle,
watching a video, giving a presentation, or sitting in a video call can still
trigger the idle timeout and lock or blank the screen. When the compositor
exposes `zwlr_foreign_toplevel_manager_v1`, dozed suppresses idle timeout
commands while any toplevel window is fullscreen.

dozed also drops the systemd dependency. Login1 D-Bus hooks (sleep, resume,
lock, unlock) are enabled by default whenever the dbus development files are
available, and work with any logind-compatible implementation.

## Installation

### Compiling from Source

Install dependencies:

- meson \*
- rustc \*
- wayland
- wayland-protocols \*
- dbus (for login1 hooks)
- [scdoc](https://git.sr.ht/~sircmpwn/scdoc) (optional: man pages) \*
- git \*

_\* Compile-time dependency_

Run these commands:

    meson build/
    ninja -C build/
    sudo ninja -C build/ install

This builds with login1 support by default when dbus is available. To build
without sleep, resume, lock, and unlock hooks:

    meson setup build/ -Dlogind=disabled
    ninja -C build/
    sudo ninja -C build/ install

## License

dozed is licensed under the [GNU General Public License v3.0](./LICENSE).
swayidle, on which dozed is based, is licensed under the MIT License.
