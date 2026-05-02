# mangoidle

This is sway's idle management daemon, mangoidle. It is compatible with any
Wayland compositor which implements the
[ext-idle-notify](https://gitlab.freedesktop.org/wayland/wayland-protocols/-/tree/main/staging/ext-idle-notify)
protocol. See the man page, [mangoidle(1)](./mangoidle.1.scd), for instructions
on configuring mangoidle.

## Installation

### Compiling from Source

Install dependencies:

* meson \*
* wayland
* wayland-protocols \*
* [scdoc](https://git.sr.ht/~sircmpwn/scdoc) (optional: man pages) \*
* git \*

_\* Compile-time dependency_

Run these commands:

    meson build/
    ninja -C build/
    sudo ninja -C build/ install
