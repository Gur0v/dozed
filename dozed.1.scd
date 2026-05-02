dozed(1)

# NAME

dozed - lightweight idle manager for Wayland compositors

# SYNOPSIS

*dozed* [options] [events...]

# OPTIONS

*-C* <path>
	The config file to use. By default, the following paths are checked in order:
	$XDG_CONFIG_HOME/dozed/config, $HOME/.config/dozed/config,
	$HOME/.dozed/config, /etc/dozed/config.

*-h*
	Show help message and quit.

*-d*
	Enable debug output.

*-w*
	Wait for each command to finish executing before continuing.

*-S* <seat-name>
	Specify which Wayland seat to use.

*--no-config*
	Do not load any config file. Only events specified on the command line are
	used.

*--dry-run*
	Validate configuration and command-line events, print the parsed events, and
	exit without connecting to Wayland or running commands.

*--validate-config*
	Validate configuration and command-line events, then exit.

*--print-events*
	Print parsed events, then exit.

*--ignore-fullscreen*
	Do not suppress idle timeout commands while a fullscreen app is active.

*--fullscreen-policy* <suppress|ignore>
	Choose fullscreen behavior. The default is *suppress*.

# DESCRIPTION

dozed listens for idle activity through the Wayland *ext-idle-notify-v1*
protocol and executes configured commands. It can run against any compositor
that exposes the protocol.

dozed is a fork of *swayidle*. The implementation has been largely rewritten,
but the event model and much of the behavior are inherited from swayidle.

dozed does not depend on systemd. If built with login1 D-Bus support, it
can also listen for sleep, resume, lock, and unlock events from a login1
implementation.

If the compositor exposes *zwlr_foreign_toplevel_manager_v1*, dozed will
not execute idle timeout commands while any toplevel window is fullscreen,
unless *--ignore-fullscreen* or *--fullscreen-policy ignore* is used.

This fullscreen-aware behavior is the main reason for the fork: videos,
presentations, and calls should not unexpectedly trigger lock or display-off
commands just because there was no keyboard or pointer input.

# EVENTS

*timeout* <timeout> <timeout command> [resume <resume command>]
	Execute _timeout command_ if there is no activity for <timeout> seconds.
	If *resume* is specified, _resume command_ runs when activity resumes.

*before-sleep* <command>
	When built with login1 support, execute _command_ before the system sleeps.

*after-resume* <command>
	When built with login1 support, execute _command_ after the system resumes.

*lock* <command>
	When built with login1 support, execute _command_ when the login1 session asks the
	session to lock.

*unlock* <command>
	When built with login1 support, execute _command_ when the login1 session asks the
	session to unlock.

All commands are executed through *sh -c*.

# SIGNALS

*SIGTERM, SIGINT*
	Run all pending resume commands and terminate.

*SIGUSR1*
	Immediately enter idle state.

# EXAMPLE

```
dozed -w \\
	timeout 300 'swaylock -f -c 000000' \\
	timeout 600 'wlopm --off "*"' \\
		resume 'wlopm --on "*"' \\
	before-sleep 'swaylock -f -c 000000'
```
