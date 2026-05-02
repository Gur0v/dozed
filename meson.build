project(
	'dozed',
	['rust', 'c'],
	version: '0.1.0',
	license: 'GPL-3.0-only',
	meson_version: '>=0.59.0',
	default_options: [
		'warning_level=2',
		'werror=true',
	],
)

add_project_arguments([
	'-D_POSIX_C_SOURCE=200809L',
	'-Wno-unused-parameter',
	'-Wno-unused-result',
	'-Wundef',
	'-Wvla',
], language: 'c')

wayland_client = dependency('wayland-client')
wayland_protos = dependency('wayland-protocols', version: '>=1.40')
dbus = dependency('dbus-1', required: get_option('logind'))
scdoc = find_program('scdoc', required: false)
wayland_scanner = dependency('wayland-scanner', native: true, version: '>=1.14.91')
wayland_scanner_prog = find_program(wayland_scanner.get_variable('wayland_scanner'), native: true)

wl_protocol_dir = wayland_protos.get_variable('pkgdatadir')

wayland_scanner_code = generator(
	wayland_scanner_prog,
	output: '@BASENAME@-protocol.c',
	arguments: ['private-code', '@INPUT@', '@OUTPUT@'],
)

wayland_scanner_client = generator(
	wayland_scanner_prog,
	output: '@BASENAME@-client-protocol.h',
	arguments: ['client-header', '@INPUT@', '@OUTPUT@'],
)

protos = [
	wl_protocol_dir / 'staging/ext-idle-notify/ext-idle-notify-v1.xml',
	'protocols/wlr-foreign-toplevel-management-unstable-v1.xml',
]

protos_src = []
foreach xml : protos
	protos_src += wayland_scanner_code.process(xml)
	protos_src += wayland_scanner_client.process(xml)
endforeach

conf_data = configuration_data()
conf_data.set10('HAVE_LOGIND', dbus.found())
config_header = configure_file(output: 'config.h', configuration: conf_data)

rust_args = []
if dbus.found()
	rust_args += ['--cfg', 'have_logind']
endif

executable(
	'dozed',
	[
		'src/main.rs',
		'src/wayland_bridge.c',
		protos_src,
		config_header,
	],
	dependencies: [
		wayland_client,
		dbus,
	],
	rust_args: rust_args,
	install: true,
)

test(
	'parser',
	executable(
		'dozed-parser-tests',
		[
			'src/main.rs',
			'src/wayland_bridge.c',
			protos_src,
			config_header,
		],
		dependencies: [
			wayland_client,
			dbus,
		],
		rust_args: rust_args + ['--test'],
	)
)

if not get_option('man-pages').disabled()
	mandir = get_option('mandir')
	if scdoc.found()
		custom_target(
			'dozed.1',
			input: 'dozed.1.scd',
			output: 'dozed.1',
			command: scdoc,
			feed: true,
			capture: true,
			install: true,
			install_dir: mandir / 'man1'
		)
	else
		install_data('dozed.1', install_dir: mandir / 'man1')
	endif
endif

subdir('completions')
