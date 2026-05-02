# dozed

This is a lightweight idle management daemon for Wayland compositors. It is
compatible with any compositor which implements the
[ext-idle-notify](https://gitlab.freedesktop.org/wayland/wayland-protocols/-/tree/main/staging/ext-idle-notify)
protocol. See the man page, [dozed(1)](./dozed.1.scd), for instructions
on configuring dozed.

dozed is a fork of swayidle. Most of the original C implementation has been
replaced, but the command model and a fair amount of the behavior still follow
swayidle's design.

dozed is implemented in Rust and does not depend on systemd. Optional login1
D-Bus hooks can be enabled at build time for sleep, resume, lock, and unlock
events.

When the compositor exposes `zwlr_foreign_toplevel_manager_v1`, dozed
suppresses idle timeout commands while any toplevel is fullscreen.

The main reason for the fork is fullscreen-aware idle behavior. With swayidle,
watching a video, presenting, or sitting in a Zoom meeting could still hit the
normal idle timeout and lock or blank the screen. dozed treats fullscreen apps as
a signal that the user likely does not want idle actions to fire.

## Installation

### Compiling from Source

Install dependencies:

* meson \*
* rustc \*
* wayland
* wayland-protocols \*
* dbus (optional, for login1 hooks)
* [scdoc](https://git.sr.ht/~sircmpwn/scdoc) (optional: man pages) \*
* git \*

_\* Compile-time dependency_

Run these commands:

    meson build/
    ninja -C build/
    sudo ninja -C build/ install
